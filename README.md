<div align="center">

# Project Waddle

A desktop AI agent you can watch. Waddle is a pixel-art duck that walks your windows, says what it is doing, and asks before anything risky.

[![CI](https://github.com/DimeDataCloud/Open-Waddle/actions/workflows/ci.yml/badge.svg)](https://github.com/DimeDataCloud/Open-Waddle/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>

Waddle is a 16 by 14 pixel duck that lives on a transparent overlay above your windows. It walks along title bars to whatever it is about to touch, explains each step in a speech bubble, and waits for your approval before anything risky. You can talk to it by typing or by voice while it works.

Waddle runs on your own machine and is free and open source under the MIT license. Model calls are optional and cheap: a busy day on a cloud model costs a few cents, and a local model costs nothing.

## Features

- **Google Workspace.** Reads and acts on Gmail, Google Calendar, Contacts and Drive. Taps you on the shoulder before meetings and when important mail arrives.
- **Chrome.** Reads and acts on web pages through the Waddle for Chrome extension, using the page's buttons, links and fields rather than screenshots.
- **Files.** Finds and reads your documents, including PDF, Word, PowerPoint, Excel and CSV, and creates new Word, Excel, CSV and Markdown files.
- **Routines.** Runs tasks on a schedule, such as "every weekday at 8:45, summarise my unread email".
- **Voice.** Push to talk, with spoken replies available. Works with the built-in Windows voice typing, or a Whisper-compatible server.
- **MCP.** Uses the tools of other apps through Model Context Protocol servers.
- **Budget.** Keeps paid model calls under a monthly limit you set, and shows what was spent on tasks, chat, research and decisions.
- **Safety.** Four permission tiers, an approval step for risky actions, an undo window for sent mail, and a tamper-evident activity log. See [Safety model](#safety-model).

## Download

Windows installers are on the [Releases page](https://github.com/DimeDataCloud/Open-Waddle/releases/latest):

- `Waddle_<version>_arm64-setup.exe` for Snapdragon PCs
- `Waddle_<version>_x64-setup.exe` for Intel and AMD PCs

Windows builds are not code-signed yet, so SmartScreen warns on first run. Choose **More info**, then **Run anyway**. See [Code signing policy](#code-signing-policy).

The Chrome extension is not yet listed in the Chrome Web Store. Until it is, install it from a file (see [Connect Google and Chrome](#connect-google-and-chrome-optional)). Publishing steps are in [docs/CHROME_STORE.md](docs/CHROME_STORE.md).

## How it works

```
            ┌──────────────────────────── Brain (waddle-core) ───────────────────────────┐
 you ──────►│ Session: one task at a time, but you can keep talking                      │
 (voice/    │   ├─ Planner lane (System 2): streams narration, calls tools, step by step │
  typing)   │   ├─ Quick-reply lane (System 1): answers instantly while the planner works│
            │   └─ "stop"/"wait"/Esc: halts immediately, no model involved               │
            │ Safety: deterministic tiers 0-3 · untrusted-content tags · hash-chained log│
            └───────┬───────────────────────────────┬────────────────────────────────────┘
                    │ GUI actions                    │ files / commands
            ┌───────▼─────── Body + Hands + Eyes (Tauri app) ─┐   ┌─▼─ Workspace folder ─┐
            │ overlay · click-through · window platforms      │   │ read/write/list      │
            │ walk → stand beside target → act (enigo)        │   │ run_command (timeout,│
            │ Windows UI Automation · screenshots (xcap)      │   │ output cap, kill)    │
            └─────────────────────────────────────────────────┘   └──────────────────────┘
```

- **Brain** (`crates/waddle-core`): plans tasks, routes each message, and applies the safety tiers. It has no operating system code and is unit tested.
- **Body, hands and eyes** (`src-tauri`, `src`): the Tauri desktop app. It draws the overlay, moves the duck, clicks and types, reads window and UI Automation trees, and takes screenshots.
- **Workspace**: a folder (`Documents\Waddle`) where Waddle reads, writes and runs commands. Commands run with a timeout and an output cap. This is not an operating system sandbox.

## Quick start (Windows 11)

Waddle is built and tested first on Windows 11 on ARM. x64 PCs get a build too. macOS and Linux run from source with fewer features: there is no UI Automation or built-in voice typing, and spoken replies use the system voices.

### Option A: install a build

1. Download the installer from the [Releases page](https://github.com/DimeDataCloud/Open-Waddle/releases) that matches your PC (see [Download](#download)).
2. Run it. If SmartScreen warns, choose **More info**, then **Run anyway**.
3. Waddle appears at the bottom of the screen with a short welcome. Paste an OpenRouter key or a Google Gemini key and press **Test**, let it find Ollama, or try demo mode.

### Option B: build from source

1. Install [Rust](https://rustup.rs). Keep the default host: `aarch64-pc-windows-msvc` on ARM, `x86_64-pc-windows-msvc` on Intel and AMD.
2. Install **Visual Studio 2022 Build Tools** with "Desktop development with C++", the **MSVC ARM64 build tools** (on ARM PCs), and a Windows 11 SDK.
3. Install [Node.js 22 LTS](https://nodejs.org) (ARM64 on ARM PCs).
4. Then run:

   ```powershell
   git clone https://github.com/DimeDataCloud/Open-Waddle
   cd Open-Waddle
   npm install
   npm run tauri dev       # run it
   npm run tauri build     # build an installer into target\release\bundle\nsis
   ```

WebView2 ships with Windows 11, so nothing else is needed.

## Choose a model

Right-click the duck, then choose **Settings**.

| Option | Cost | Notes |
|---|---|---|
| **OpenRouter** (default) | About $0.0005 to $0.001 per task (measured in [docs/MODELS.md](docs/MODELS.md)) | Create a key at <https://openrouter.ai/keys> and add credit. Defaults: `openai/gpt-6-luna` plans, `google/gemini-2.5-flash-lite` chats and researches, and the Jev model makes the quick decisions. |
| **Google Gemini API** | Free tier, or pennies | Get a key at <https://aistudio.google.com/apikey>. `gemini-3.5-flash-lite` plans and chats. On the free tier, Google may use what you send to improve its products, so turn on billing for private use. Web search and the quick chat lane need OpenRouter. |
| **Ollama** (local) | Free | Install [Ollama](https://ollama.com) 0.17.6 or newer, then run `ollama pull qwen3.5:4b`. Waddle turns off hidden thinking, limits Ollama to a third of the CPU cores and unloads the model 30 seconds after each task. On Snapdragon it runs on the CPU, so it is slow. `qwen3.5:2b` is faster; `qwen3.5:9b` needs about 32 GB of RAM. |
| LM Studio, Foundry Local or custom | Free | Any OpenAI-compatible endpoint. Foundry Local can use the Snapdragon NPU for text models. |

**Use both for the best value.** Settings, then Brain, then **Quick replies run on**, set to **Google Gemini API**, puts chat, research summaries and the morning brief on Google with its own key. Turn on **Run tasks on Gemini too** to run tasks there as well. OpenRouter keeps the quick decisions, the screen check and web search. If Google refuses a request over its free-tier limits, OpenRouter answers instead. The **Test** button makes a real tool call, so you can confirm a setup before relying on it. See [docs/LAB.md](docs/LAB.md) for the model comparisons.

API keys are stored in Windows Credential Manager (macOS Keychain or Linux Secret Service), not in files.

## Connect Google and Chrome (optional)

- **Google** (Gmail, Calendar, Contacts, Drive): create your own OAuth client by following [docs/GOOGLE_SETUP.md](docs/GOOGLE_SETUP.md), paste it into Settings, then Google account, and press **Connect**.
- **Chrome**: Settings, then Assistant, then Chrome. Press **Get Waddle for Chrome** if the store listing is live, or **Install from a file** to load the extension in Developer mode. See [extension/README.md](extension/README.md).
- **Files**: Settings, then Files. Waddle can read your user folders. Add the folders it may also change.

## Features in detail

### On your desktop

- Waddle walks along the top edges of windows and the bottom of the screen, rides windows you drag, and falls when its window disappears.
- It wanders when idle and falls asleep after two quiet minutes.
- Drag the duck to carry it. Click it to chat. Right-click it for Settings.
- With more than one monitor, it follows you to the screen you are working on, never in the middle of a task. It re-places itself when you rotate, dock or change the scale.
- Only one Waddle runs at a time. On battery it checks the screen less often.
- Small effects show the outcome: a sparkle when a task is done, a sweat drop when one fails, a question mark when it needs an answer, and a heart when you say thanks. Reduce Motion turns these into symbols only.

### Conversation

- Replies stream into the bubble as they are generated.
- Messages you send during a task get an instant quick reply and change the running task at its next step.
- Saying or typing "stop", "wait" or "cancel" halts at once.
- The history button in the chat box, or **Ctrl+Alt+H**, opens the last 50 messages beside the duck. Long replies show their start with a **More** button.
- Up arrow in the chat box recalls your previous message. Shift+Enter starts a new line.

### Voice

- Press **Ctrl+Alt+Space**, or the microphone button, to talk.
- On Windows this uses the built-in voice typing, which is free and live.
- A Whisper-compatible backend is also available: Groq's free tier, OpenAI, or a local whisper server.
- **Spoken replies** (Settings, then Voice, off by default) read answers aloud in a Windows voice of your choice. Only answers are read, not every step. Waddle stays quiet in full screen and during meetings on your calendar, and stops when you open the chat box, speak or press Stop.
- **Talk mode** listens for 6 seconds after an answer to something you said aloud, so you can hold a conversation without touching the keyboard.

### Acting on your computer

- Lists windows. On Windows it reads the buttons and fields of any window through UI Automation, and takes screenshots when needed.
- Opens apps, clicks, types, presses shortcuts, scrolls and drags. Before each action it walks to the target and stands beside it.
- Arranges windows in one step: "GitHub on the left, YouTube Music on the right", "put that on my right monitor", "maximize Word".
- Matches misheard app names to installed apps by spelling and sound. When it is unsure, it asks.
- Plans several clear steps at once and goes straight to search results on YouTube, YouTube Music, GitHub, Google, Maps, Wikipedia and Amazon.

### Answer, research or act

A small decision model sorts each message in about 0.3 seconds:

- **Chat** ("thanks!", "what is a haiku?") gets an instant answer without a task. Questions that need current facts use a web search.
- **Research** ("research how to keep basil alive indoors") gets a short cited summary in the bubble. The **Full answer** button saves the whole answer, with numbered sources, to `Documents\Waddle\research\` and opens it.
- **Tasks** start straight away. Waddle looks at the screen while the router decides.

### Google

- "What's my next meeting?" answers in a few seconds from your calendar, without touching the screen.
- "Any important unread email?", "archive the newsletters", "reply to Ana that Thursday works", "find 45 minutes with Sam next week and invite him".
- Before anything is sent, a card shows the whole message. You can edit it, and after **Send** you have 10 seconds to **Undo**. Deleting mail and events also waits for your click.
- "Learn how I write emails" builds a short style note from about 20 of your sent messages, so drafts sound like you.
- Without a Google connection, Waddle uses Gmail and Calendar in Chrome on screen instead.

### Nudges and the morning brief

With Google connected, Waddle:

- Reminds you five minutes before a meeting, with **Join** (the Meet link) and **Snooze**.
- Flags important mail with **Open**, **Reply** and **Mute sender**. It judges importance from the sender, subject and first line only.
- In full screen, shows a small "!" and waits until you return or five minutes pass. Meetings always arrive before they start.
- Offers a brief the first time you use the computer after 6 am: today's meetings and important unread email in one short paragraph.
- Starts with Windows (switch this off in Settings), so nudges work without opening the app.

### Files

- Finds files by name, then by text inside them, in Documents, Downloads, Desktop, Pictures, Music, Videos and Google Drive.
- Reads PDF, Word, PowerPoint, Excel and CSV files as text. With Google connected it also reads Drive files, including Docs, Sheets and Slides.
- Creates and changes files only in its workspace and in folders you allow under Settings, then Files. Each change shows a 2-second notice.
- Deleting sends items to the Recycle Bin and waits for your click. Overwriting keeps the old copy in the Recycle Bin.
- App data, SSH keys, password databases, browser profiles and `.env` files are always off limits.

### Chrome

- Reads the page text, or its buttons, links and fields, and acts on them by name.
- Clicks are real mouse clicks. The duck walks to the button and presses it, so pages behave as they would for you.
- "Search for..." goes straight to Google's results and reads them.
- Without the extension, Waddle still drives Chrome from the screen.

### Routines and MCP

- **Routines** run on schedule, for example "every Friday at 4pm, list what I worked on this week". They run without the screen where possible and ask before changing anything. A routine that was missed while Waddle was closed is skipped, not run late. Manage them under Settings, then Nudges and routines.
- **MCP** servers add tools from other apps, such as GitHub, Notion or a database. Add them under Settings, then Assistant, then Tools (MCP), by typing the command or pasting the JSON block the server's instructions show. Untrusted servers wait for your click, and tool output is treated as untrusted text. See [docs/MCP.md](docs/MCP.md).

### Other features

- **Work on a selection.** Select text anywhere and press **Ctrl+Alt+A**. The chat opens with the selection attached. The result goes to the clipboard, or after a 2-second notice replaces the selection.
- **Memory.** "Remember that I take my coffee black" saves a fact. The list holds up to 2 KB and every later task can see it. Waddle may save useful facts on its own, never from the screen, files or web pages. Edit the list under Settings, then Memory and privacy.
- **Budget.** Settings, then Brain, shows today's and this month's spending by day and by purpose. The default monthly limit is $5. Waddle warns at 80% and pauses paid model calls at 100% until the first of the month. Local models are never counted or paused.
- **Idle behaviour.** A small decision model picks what the duck does between tasks, based on the app you are in and whether you are idle. It never reads window titles.
- **Assist.** "Where is the export button?" makes Waddle circle the spot with a label instead of clicking. It reads what you copied and can put results on the clipboard. Reminders ("remind me in 20 minutes to stretch") survive restarts.
- **Learning.** When a task goes well, Waddle keeps a short recipe of its steps: tool and app names, never typed text or page content. It reuses the recipe for similar requests. Thumbs up confirms a recipe, thumbs down forgets it, and **Forget conversation** clears them all.
- **Self-improvement, with approval.** Waddle can save skills that it reads back at the start of each task, change its own settings, split large jobs into sub-tasks (two levels deep by default), and edit its own source code if you point it at this repository.

## Safety model

| Tier | What | What happens |
|---|---|---|
| 0 | Reading the screen or a web page, pointing at things | Runs silently |
| 1 | Opening apps and web pages, switching tabs, scrolling, finding and reading files and Drive, reading the clipboard, reading mail, calendar and contacts, reminders, remembering facts, starting sub-tasks | Runs, and is logged |
| 2 | Clicks and typing, closing tabs, dragging, copying to the clipboard, replacing a selection, new files, overwriting, moving and renaming files, read-only commands such as `dir` or `git status`, mail drafts, archiving and labels, new or changed events, invitations | Shown with a 2-second countdown and a **Cancel** button, or always asks if set in Settings |
| 3 | Deleting (to the Recycle Bin), other commands, network access, self-changes (skills, settings, source code), sending email, binning mail, deleting events | Waddle turns red and waits for your click |

- **Tiers are fixed rules, not a model's judgement.** Approval only comes from a click in Waddle's own interface, never from model output.
- **Screen and file contents are data.** Text from the screen, files and command output is wrapped in tags with an unguessable id, and the model is told not to follow instructions inside them.
- **Stopping.** Press Escape (active only while a task runs), the Stop button, double-click the duck, use the tray menu, or say "stop". Waddle reports which method stopped the task and offers **Try again**.
- **Activity log.** Every action, decision and result is written to an append-only SQLite log. Each entry is hash-chained to the one before it, so edits are detectable. Verify it under Settings, then Activity log, then **Verify integrity**.
- **No training on your data by default.** OpenRouter requests ask for providers that do not keep or train on prompts (`data_collection: deny`). You can turn this off under Settings, then Memory and privacy, if a model you want needs it.
- **Limits.** Commands run in the workspace folder, but this is not an operating system sandbox. The tier 3 approval is the real gate.

## Configuration

Settings are managed in the app. These environment variables are read at startup. Values are never stored in the repository.

| Variable | Purpose |
|---|---|
| `OPENROUTER_API_KEY` | OpenRouter key, for development. The app keeps keys in the credential store. |
| `WADDLE_API_KEY` | Model key, checked before `OPENROUTER_API_KEY`. |
| `GEMINI_API_KEY` | Google Gemini API key, used by the Gemini provider and the test suite. |
| `WADDLE_PROVIDER` | Set to `mock` to force demo mode. |
| `WADDLE_OLLAMA_URL` | Ollama server URL for tests and benchmarks. Default `http://localhost:11434`. |
| `WADDLE_LIVE`, `WADDLE_LIVE_MODEL`, `WADDLE_LIVE_FAST_MODEL` | Opt-in live model tests and the models they use. |
| `WADDLE_BENCH_*`, `WADDLE_LAB_*` | Benchmark and click-lab settings used by the test binaries in `crates/waddle-core/tests`. |
| `WADDLE_GOOGLE_BASE` | Points Google requests at a fake server on this machine, for development. |
| `WADDLE_BROWSER_SOCKET` | Overrides the socket or pipe the Chrome extension relay connects to. |
| `WADDLE_TRACES`, `WADDLE_EXPORT` | Folders for the traces export test (`crates/waddle-core/tests/traces_export.rs`). |

## Something not working?

Right-click the duck, then choose **Settings**, then **Diagnostics**, then **Run self-test**.

- The self-test checks the display and monitors, window tracking, accessibility, screenshots, the mouse and keyboard, key storage, the workspace, commands, the activity log, the talk shortcut, the microphone, MCP servers, routines, spoken-reply voices, the spending budget and the model (one small call).
- It saves `waddle-report-<time>.txt` to your workspace folder. The report leaves out keys, file contents, window titles and what you typed.
- The log is at `%LOCALAPPDATA%\dev.waddle.app\logs\waddle.log`. On macOS it is `~/Library/Logs/dev.waddle.app/`, and on Linux `~/.local/share/dev.waddle.app/logs/`.

## Project structure

```
crates/waddle-core/   Brain, non-GUI hands and safety (no OS code; unit tested)
  src/session.rs        real-time session: steering, quick replies, halt phrases, memory
  src/agent.rs          planner loop, tier gate, delegation (recursive sub-tasks)
  src/llm/              streaming providers: OpenAI-compatible (SSE), Ollama (NDJSON), mock
  src/safety.rs         deterministic permission tiers
  src/skills.rs         long-term skill memory
  src/audit.rs          append-only, hash-chained SQLite log
  src/tools/            tool schemas, coordinate conversion, scoped files, documents, browser tools, commands
  src/google/           Google sign-in (PKCE), Gmail, Calendar, Contacts, Drive
  src/nudges.rs         when to nudge: meetings, important mail, the morning brief
src-tauri/            Desktop shell: overlay, click-through, window tracking, input, UI Automation, voice, tray
src/                  Overlay frontend: sprite renderer, palette maths, physics, pathfinding, UI
extension/            Waddle for Chrome (Manifest V3): reads pages and acts on elements via native messaging
bench/                Model benchmark: app screens rendered in Chrome, with task checks
training/             Synthetic click data, LoRA fine-tuning and GRPO scripts
scripts/              Icon, store art, extension packing and update manifest scripts
docs/                 Architecture, models, lab, Google setup, MCP, training, roadmap, Chrome Web Store
```

## Development

```bash
cargo test --workspace       # core and app unit tests, agent and session integration tests
npm test                     # frontend and the Chrome extension page code (Vitest, jsdom)
npm run typecheck            # TypeScript for the app, the Node side and the extension
npm run build:extension      # rebuild the extension; its built files are committed and embedded in the app
npm run tauri dev            # run the app in development mode
```

Linux build dependencies: `libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libpipewire-0.3-dev libasound2-dev libclang-dev`.

Architecture notes are in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Training scripts are described in [docs/TRAINING.md](docs/TRAINING.md).

## Status and roadmap

The current release is 0.3.2. Each release is described in [CHANGELOG.md](CHANGELOG.md). Planned work is in [docs/ROADMAP.md](docs/ROADMAP.md), and the 0.3 plan and evidence are in [docs/BLUEPRINT-0.3.md](docs/BLUEPRINT-0.3.md).

Open items from the roadmap:

- Testing on Surface hardware with the on-device checklist (see below).
- A code-signing certificate, to remove the SmartScreen warning.
- A Chrome Web Store listing for the extension.
- macOS support: accessibility and screen-recording prompts, and an Apple Silicon build.
- Foundry Local NPU mode, and a fine-tuned local planner.

### On-device checklist

The checklist for testing on a real Windows 11 PC is at the end of this section. Each step says what to do and what you should see. Steps marked (G) need Google connected, (C) the Chrome extension, and (2M) a second monitor.

1. Start Waddle with no settings. Paste an OpenRouter key and press **Test**, which should report "It works". Choose a colour and press **Let's go**.
2. Start Waddle a second time. No second duck appears.
3. The duck drops onto a window's title bar or the taskbar edge and wanders.
4. Clicks pass through the overlay everywhere except on the duck and its bubbles.
5. Drag the duck high and drop it. It falls and lands on a window with a puff of dust.
6. Rotate the screen, or dock and undock the PC. Within 2 seconds the duck is back on screen.
7. (2M) Focus a window on the other monitor. The duck walks in from that side.
8. Unplug the charger. The duck keeps walking smoothly, and Task Manager shows less CPU use while idle.
9. Ask "Create hello.txt that says hi". A Tier 2 countdown appears, then `Documents\Waddle\hello.txt` exists.
10. Ask "Delete hello.txt". The duck turns red and an approval card waits. Press Escape to deny it. Typing Enter in the chat box never approves.
11. Ask "Open Notepad and type hello". Waddle opens Notepad, walks to it and types.
12. While it works, say "actually type goodbye instead". You get an instant reply and the task adapts.
13. Press Escape during a task. It stops with "Stopped (you pressed Esc)" and offers **Try again**.
14. Settings, then Activity log. Each step is listed, and **Verify integrity** reports the log intact.
15. Ask for research on a topic. You get a short summary with sources, and **Full answer** opens a Markdown file in `Documents\Waddle\research\`.
16. Ask for a long answer. The bubble shows its start and **More**. **Ctrl+Alt+H** opens the history beside the duck.
17. Say "thanks!". A heart appears by the duck.
18. Select a sentence in Notepad, press **Ctrl+Alt+A**, and type "make this more formal". After a 2-second notice the sentence is replaced.
19. Ask Waddle to remember a fact. It appears under Settings, then Memory and privacy, and a later question can use it.
20. Press **Ctrl+Alt+Space** and ask "what's 12 times 7?". Your words appear as you speak, then the answer.
21. Turn on **Spoken replies**, choose a voice and save. Ask a question and the answer is read aloud. Open the chat box while it speaks and it stops.
22. (G) Connect Google. The settings show "Connected as" your account.
23. (G) Ask "What's my next meeting?". The answer arrives in a few seconds, with the Meet link if there is one.
24. (G) Ask Waddle to reply to an email. The send card shows the whole reply. Press **Undo** within 10 seconds and nothing is sent.
25. (C) Install the Chrome extension. Settings shows "Connected (extension ...)" within a minute.
26. (C) On a search page, ask Waddle to fill in the search box and press search. The page reacts as if you had clicked.
27. Set the monthly budget to $0.01 and run a couple of tasks. Waddle warns at 80%, then pauses paid model calls.
28. Settings, then Diagnostics, then **Run self-test**. Every line should show Pass or Skip. Warnings say why.

Keep a note of anything that fails, with the report file from the self-test.

## Code signing policy

Free code signing for Windows releases is applied for through [SignPath.io](https://signpath.io), with the certificate from the [SignPath Foundation](https://signpath.org). **Until that is approved, releases are not signed.** Once approved, every installer will be signed this way.

> Free code signing provided by [SignPath.io](https://signpath.io), certificate by [SignPath Foundation](https://signpath.org).

- **What is signed:** only Windows installers built by this repository's [release workflow](.github/workflows/release.yml) on GitHub-hosted runners, from the source in this repository.
- **Team roles:** the maintainer, [@DimeDataCloud](https://github.com/DimeDataCloud), is the author, reviewer and approver. Every release is approved by hand before it is signed. Everyone with these roles uses multi-factor authentication.
- **Privacy:** see [PRIVACY.md](PRIVACY.md). Waddle sends nothing to the project or to anyone else, except the requests you set up (your chosen model provider, and Google if you connect it) and the update check when you press it.

## Contributing

Bug reports and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md). To report a security problem privately, see [SECURITY.md](SECURITY.md).

## License

Waddle is released under the [MIT license](LICENSE).
