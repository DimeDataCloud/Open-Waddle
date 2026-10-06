# Project Waddle architecture (MVP)

This maps the Technical Blueprint and Competitor Analysis onto what is built. It also records what was deferred, and why.

## Constraints that shaped the MVP

- **Hardware:** the first machine is a Surface Pro on Windows 11 ARM64 (Snapdragon), with no discrete GPU and shared RAM. macOS comes next; Linux X11 builds work (CI and development).
- **Cost:** about zero. No servers; you bring your own model key, or run a local model.
- **Headroom:** the user must be able to run heavy work alongside Waddle. The overlay idles at near-zero CPU, and local inference is throttled.
- **Responsiveness:** the user talks to Waddle while it works, gets text back within about a second, and can redirect or stop it at any time.
- **Self-modification and recursion are allowed,** with a human gate on every change Waddle makes to itself.

## Layers

### Body (`src/body/*`, `src-tauri/src/overlay.rs`, `src-tauri/src/desktop/*`)

- **Overlay window:**
  - one transparent, borderless, always-on-top Tauri 2 window over the primary monitor's **work area** (not the full screen, so Windows doesn't treat it as a full-screen app)
  - hidden from screen capture (`contentProtected`), so screenshots sent to the model don't show the duck
- **Click-through:**
  - the frontend reports the rectangles it draws: the duck, the bubble, the chat box and the approval card
  - a Rust thread polls the cursor every 25 ms and toggles `set_ignore_cursor_events` only when the state changes
  - during synthetic clicks the overlay is forced click-through, so Waddle can't click itself
- **Window tracking at 10 Hz:**
  - Windows: native EnumWindows (z-order), DWM extended frame bounds, cloaked-window filter, process names
  - macOS/Linux: the `x-win` crate
  - an event goes out only when something changed
- **Monitors:** the overlay covers one monitor's work area at a time. A thread checks every 2 s: display changes (rotation, docking, scale, taskbar) re-place it at once, and with "follow me" on it moves to the monitor of the focused window after two sightings, never mid-task. Screenshots, window and element coordinates and clicks all use that monitor's geometry. On Windows, pointer moves off the primary monitor use `SendInput` with `MOUSEEVENTF_VIRTUALDESK`, because enigo scales absolute moves to the primary only.
- **Platforms:** the visible parts of each window's top edge (windows in front hide parts of the edges behind them), plus the floor.
- **Physics:** gravity; landing on the first edge crossed; riding a window that moves (tracked by window id); falling when its edge disappears.
- **Pathfinding:**
  - shortest-path search over 8 px standing columns on the surfaces
  - moves: walk (cost 1 per column), step off an end and drop (cheap), jump (cost grows with height, capped at a 96 px leap)
  - the last leg flies when the target isn't on a surface (ducks fly)
  - trips are capped at 2.5 s when approaching and 1.2 s when acting
  - Waddle stops **beside** the target, beak first
- **Rendering:**
  - 16×14 frames stored as text grids (`waddle.sprites.json`), drawn at 4× with no smoothing
  - every colour derives from one base colour: dark = base × 0.62, darkest = base × 0.36
  - red variant while a Tier 3 approval waits
  - full frame rate only while something animates; an 8 fps tick otherwise
- **Effects** (`src/body/fx.ts`): plain canvas shapes drawn after the duck: dust on landing after a real drop, a sparkle when a task is done, a sweat drop when one fails, a "?" when a reply ends with a question, a heart when thanked. Each frame clears only the area the last one drew. With `prefers-reduced-motion`, dust is skipped and the rest stay still.
- **Approval card:** announced to screen readers (an `alertdialog` when it needs a click, a polite `status` during a countdown). Esc denies; there is deliberately no Enter shortcut to approve, and the card never takes keyboard focus on its own, so typing elsewhere can't approve anything.

### Eyes

| Path | Built | Notes |
|---|---|---|
| Window list | ✅ all platforms | titles are untrusted text |
| Accessibility fast path | ✅ Windows UI Automation | interactive, enabled, on-screen elements; depth 12, 150 elements, 1.5 s budget; dedicated COM thread; `click_element` invokes the element when it supports it, else clicks its centre |
| Screenshot slow path | ✅ xcap | downscaled to logical resolution; only when the model asks; only the latest image is kept in history |
| macOS accessibility | ⏳ | next milestone |
| OmniParser v2 | ⏳ deferred | needs a GPU and runs in Python; its detector is **AGPL** (the blueprint says MIT) |

**Coordinates:** Rust uses physical pixels; the model sees logical pixels (Surface: 2880×1920 at 200% = 1440×960). Each model gets either a 0–1000 grid (Qwen, Gemini, Gemma) or pixels (Claude, GPT), auto-detected from the model name and overridable in Settings.

### Hands

- **GUI input (enigo 0.6):** `click`, `click_element`, `type_text`, `press_keys`, `open_app`.
- **Windows arranged through the OS** (`arrange_window`, tier 1): the geometry is worked out in the core (`tools/arrange.rs`: the monitor holding the window's centre, or the one asked for, and that monitor's work area from Tauri), and the app applies it: on Windows `SetWindowPos` sized so the visible frame (DWM extended bounds, without the invisible resize borders) fills the target, `ShowWindow` to maximize, minimize or restore, then `SetForegroundWindow`; minimized windows are found too. On Linux it's `xdotool`.
- **The duck's own tricks** (`duck`, tier 0): the overlay plans the moves (`src/body/tricks.ts`: fly a loop, come beside the pointer, dance, nap, wake, hide, a pretend mess with dust) and reports back when done, like a walk.
  - Each walks the duck there first, both before the approval gate and before acting.
  - The user's cursor is put back afterwards.
  - Keyboard focus returns to the user's last app before typing.
  - `open_app` on Windows uses ShellExecuteW (no shell parsing), then a fuzzy match over Start Menu shortcuts.
- **Files and commands (core):** `read_file`, `write_file`, `list_dir`, `run_command`.
  - Paths are confined to the workspace via lexical normalisation plus symlink resolution.
  - Commands run in PowerShell (Windows) or `sh`, with a timeout, a 64 KB output cap, line streaming to the bubble, and process-tree kill on halt or timeout.

### Brain (`crates/waddle-core`)

- **Providers (streaming):**
  - OpenAI-compatible SSE: OpenRouter, OpenAI, LM Studio, llama.cpp, Foundry Local
  - native Ollama NDJSON, with `num_thread`, `keep_alive` and `num_ctx` controls
  - a mock provider for tests and demo mode
  - tool calls written as text (`<tool_call>` blocks from local Qwen servers) are recovered
  - rate limits, server errors and dropped connections are retried twice on the same model (1 s, then 3 s, or what `Retry-After` asks for, up to 8 s), but only before any words have reached the user; a stream silent for 60 s (300 s for local servers) counts as dropped
  - a reply that stops at the token limit is flagged, and its tool calls are answered "cut off" instead of being run
- **Planner (System 2):**
  - the agent loop: stream → for each tool call: approach → classify the tier → gate → act → wrap untrusted output → feed back
  - limits: 20 steps per task; a step budget shared across sub-tasks; a 5-minute task timeout
  - every wait races the halt token
- **Quick-reply lane (System 1):**
  - a message sent mid-task gets a streaming reply with no tools, from a fast model, built from a live status snapshot (goal, step, narration)
  - the same message is folded into the planner at the next step, with a note that it was already answered
- **Harness:** every task starts with the window list, accessibility elements (Windows) and a screenshot attached to the request (`look_first`). `click_element` names are checked against ids, and a third identical click in a row is refused. Measured on `bench/`: 27% → 93% for the default model ([MODELS.md](MODELS.md)).
- **Training traces** (opt-in): finished tasks are saved with their screenshots, rated 👍/👎 in the bubble, and exported as fine-tuning data ([TRAINING.md](TRAINING.md)).
- **Halt phrases** ("stop", "wait", "cancel"…) are matched by fixed rules and stop the task without any model call.
- **Esc while a panel is open:** the overlay reports when the chat box or history drawer opens or closes (`panels_open`). While a task runs, Escape is a global hotkey, so the webview never sees it; with a panel open, the first press closes the panel (`ui:dismiss`) and only the next stops the task. A stopped task shows **Try again**, which sends the same request.
- **Conversation memory:** the last 10 exchanges carry over between tasks, and between runs (`memory.json`, forgotten after 12 hours of quiet).
- **Routines** (`routines.rs`, kept beside the reminders in `routines.json`): a goal, a local time and days (a weekday mask, or a one-off date). The app's 5-second reminder clock calls `take_due`: a routine up to 15 minutes late starts when the session is free (one at a time), and one later than that is marked missed and moved to its next time. `Session::run_routine` starts it as a normal task with the screen, accessibility and Chrome capabilities off, no opening look, tier 2 set to always ask, and a prompt note that the user may be away; the outcome is written back to the routine. Adding one through the `routine` tool is tier 3 (it stores instructions that run later); listing, pausing and deleting are tier 1. Routines can't create routines.
- **Only offered tools run:** the agent remembers the tools it offered for the task and refuses any other name without running it (a made-up tool, the screen inside a routine, `delegate` past the depth limit).
- **MCP tools** (`mcp.rs`): a stdio JSON-RPC client (newline-delimited, `initialize` with protocol 2025-11-25, accepting servers that answer 2024-11-05 to 2025-11-25, then `notifications/initialized`, paged `tools/list`, `tools/call`). `McpHub` keeps each server's tool list in `mcp_tools.json` (keyed by a fingerprint of its command, arguments and variable names), offers them as `mcp_<server>_<tool>`, starts a server on its first call (a server found dead is started again; a call it dies during isn't retried), and stops servers idle for 10 minutes. Timeouts: 15 s to start or list, 60 s per call, all racing the halt token. Results are untrusted (`mcp`), text only, 16 KB at most. Tiers come from `safety::classify_mcp`: a server's read-only hint lowers the tier only when the user marked it trusted. Servers are user-only settings (`mcp_servers`); their variable values are one base64 JSON entry in the keychain. On Windows, commands resolve through PATHEXT (`npx` → `npx.cmd`) and start without a console window.
- **Spoken replies:** `speech.rs` in the core picks the words (chat and research answers and quick replies as they finish; a task's last line only when it ends; failures but not stops) and cleans them for a voice (no Markdown, emoji, citations or web addresses; about 480 characters at most, cut at a sentence). The app's `speech.rs` speaks on one thread with a queue: WinRT `SpeechSynthesizer` → `MediaPlayer` on Windows, `say` on macOS, speech-dispatcher or eSpeak on Linux. It keeps quiet in full screen and while a calendar event is on (from the meeting-nudge poll), and stops on a new message, the chat box opening, voice input or halt. Talk mode reopens the mic after an answer to a spoken message.
- **Conversation history** (`history.rs`, for the drawer, never sent to a model): the app's `Host::emit` folds each finished reply line (text deltas until `TextDone`, or a task's end) into `history.json`, with the user's messages, nudges, reminders, briefs and failed tasks; 50 entries, saved atomically. A research reply carries its full Markdown, so **Full answer** still works after a restart. The drawer renders Markdown into DOM nodes with `textContent` only (no `innerHTML`), and links open through `open_link`, which accepts http and https only.
- **Context budget:** past about 48,000 characters, earlier tool results over 1,500 characters are cut to their first 1,000 (an untrusted block keeps its closing tag), all at once like screenshots; the latest step's results stay whole.
- **Spending:** hosted providers are wrapped in `ledger::Metered`, which records each call's reported cost by purpose (a task-local set by the chat, research, quick-reply, brief and self-test call sites; tasks otherwise) in `spending.json`, and refuses calls once the month's spending reaches `monthly_budget`. Jev decisions are recorded too and skipped over budget, so every caller falls back to its careful default.
- **Local prompt cache:** within a task the conversation is append-only, so a local server (Ollama/llama.cpp) reuses its cache and only processes new tokens each step. Local models keep up to 2–3 screenshots before older ones are dropped in one go (hosted APIs keep 1, since every image is billed on every call). Opening the chat box warms a local model: it loads and reads the system prompt, tools and conversation so far while the user types, so the first step only reads their message (36.6 s → 6.8 s here on a 4-core CPU). The warm-up asks Ollama for thinking on, because the empty think block that `think: false` appends ends the prompt past Qwen3.5's last cache checkpoint.

### Assistant tools

| Tool | Tier | What happens |
|---|---|---|
| `point_at` | 0 | The duck walks to the spot and the overlay draws a pulsing ring with a label for 8 s. The ring never takes clicks. Prompted for "where is…" and "how do I…" questions. |
| `scroll` | 1 | Mouse-wheel notches at a point or under the cursor, one notch at a time so apps that smooth scrolling see real scrolling. Counts as blind input, so it needs a look first. |
| `drag` | 2 | Press, glide in 20 steps, release, so apps register a drag (files, sliders, windows). |
| `read_clipboard` / `copy_to_clipboard` | 1 / 2 | `arboard`. Clipboard text is wrapped as untrusted, since a web page can put text there. |
| `reminder` | 1 | add / list / cancel, stored in `reminders.json` in the app data folder (`reminders.rs`). A thread checks every 5 s and emits `reminder`; the overlay chimes and pins the bubble for a minute. Reminders that came due while the app was closed pop up at start, marked as missed. |

- **Claims without tools:** a final reply that says something was done ("I've opened YouTube", "Opened the repository.", "is now playing") when no tool has run in the task gets one nudge (`claims_action`): nothing changed, earlier replies may be wrong, do it now or say you can't. Small models copied such lines from earlier replies in real use.
- **Batches stop at the first failure:** a reply may make several calls (they run in order; reads that don't touch the screen run side by side). A denied or failed call skips the rest of that reply's calls, which were planned on top of it.
- **Read in the same step:** `browser_navigate`, `browser_click` and `browser_tabs` take `read: "text" | "elements"` and return the page read with their result (after a click, 600 ms later), which saves a model step each time. `browser_navigate`'s description carries direct search addresses for common sites.
- **Workflow memory** (`experience.rs`, `experience.json`, 200 recipes): a task that ends Done without asking anything back leaves a recipe of its successful calls, written only from what the model chose (tool names, app names, canonical keys, and addresses without their query, kept only for well-known sites or a site the request named); nothing from tool results goes in, so untrusted text, or an address a page talked the model into, can't come back as advice. A new request whose content words match an earlier one (Dice ≥ 0.5, at least two shared words) gets the closest recipe appended to the request, not to the system prompt, so the prompt cache holds. 👍 confirms, 👎 deletes, Forget conversation clears; the 👍/👎 question appears whenever a recipe was kept.
- **Misheard names** (`heard.rs`): `open_app` falls back to matching the installed apps (Start menu shortcuts plus `Get-StartApps` for Store apps, cached 10 minutes) by Soundex and edit distance; a clear winner opens, otherwise the error lists the closest names for the model to ask about. Spoken messages lose dictation quotes and fillers before routing.
- **Text tool calls:** Qwen sometimes writes a call as text (`point_at(x=920, y=240)`, `</tool_call>`) instead of making it. `writes_tool_call` spots an offered tool name followed by `(` or `{`, and the model gets one nudge to make the real call.
- **Quick decisions (`decide.rs`, Jev on OpenRouter, ~0.3 s and ~$0.00002 each):**
  - **Ambient behaviour (`ambient_brain`):** every 4 s the app samples the front app's name, whether it fills the screen, and idle time (cursor movement, plus `GetLastInputInfo` on Windows). Only when that changes (at most every 20 s, at least every 2.5 min) Jev picks perch / explore / watch / nap / give space / wander (`ambient.rs`). The overlay turns the choice into stops on the window's top edge, the cursor or the floor corner (`body/intent.ts`). Window titles are never sent.
  - **Chat lane (`quick_chat`):** an idle message that Jev rates ≥ 0.8 "just conversation" gets an answer from the fast model with no tools and no screenshot. If that model replies `[task]`, the message becomes a normal task.
    - A plain request for action ("uh, can you open up YouTube?", "go to GitHub and check the pull requests") skips the router and starts a task (`asks_for_action`: spoken fillers, quotes and "can you / go ahead and" are dropped, then the first word decides).
    - The reply streams a sentence at a time. A sentence that claims an action is never shown and the message becomes a task, so a false "I've opened Chrome" neither reaches the bubble nor the conversation memory.
    - The prompt says what the chat lane can't do, how Waddle is used (hotkeys, stopping, Settings) and whether Google and Chrome are connected, and asks it not to mention the time unless asked.
- **Screen check (`smart_look`):** on OpenRouter, TypeSafe's Jev decision model is asked first whether the task needs the screen (`Provider::needs_screen`). Below 0.3 the opening screenshot is skipped; the model can still look itself, and blind input stays blocked until it does.
- **Time:** the opening observation starts with the local date and time, so "at 3pm" and "what time is it" work without changing the cached system prompt.

### Self-modification and recursion

| Capability | Tool | Tier | Guard rails |
|---|---|---|---|
| Learn skills | `save_skill`, `forget_skill` | 3 | stored outside the workspace (file tools can't touch them); 4 KB per skill, 12 KB total; loaded into every system prompt; viewable and deletable in Settings |
| Tune itself | `update_settings` | 3 | only a short list of settings: model, fast model, coordinates, wander, colour, Tier 2 mode/countdown, max steps, command timeout, voice. Endpoints, keys, folders and the self-edit switch are user-only, so a hijacked task can't redirect traffic or widen its reach |
| Edit its own code | `write_file self/...`, `run_command cwd=self` | 3 for writes | off unless the user sets "Own source folder" in Settings |
| Recursive sub-tasks | `delegate` | 1 | each nested action is gated on its own; depth limit (default 2); steps shared across the whole tree; the user's Stop halts every level |

### Safety

- **Tiers come from fixed rules** (`safety.rs`):
  - Tier 3 applies to unknown tools, overwrites, every command off the read-only allowlist, shell control characters, network/destructive/secret-revealing words, and paths outside the workspace.
  - No model output can lower a tier or approve an action.
- **Untrusted content:** tool output from the screen, files and commands is wrapped as `<untrusted source=… id=RANDOM>`, and look-alike tags inside it are defanged. Screenshots are labelled as untrusted.
- **Audit log:** SQLite, append-only (triggers reject UPDATE/DELETE), each row a SHA-256 hash covering the previous row; `verify()` locates the first altered row.
- **API keys:** stored in the OS keychain (`keyring` 4); environment variables override for development; a file readable only by the user is the fallback on headless Linux.

### Two AI services

One endpoint and key serve the planner; the quick-reply model (chat, research summaries, the morning brief, mail-style learning) can have its own (`fast_base_url`, a separate key in the keychain). `llm::Routed` sits behind the one `Provider` the rest of the code sees and sends each request by its model name, so no caller knows there are two. Without a key for the second service, or for a web search it can't run (only OpenRouter's plugin can), the planner's service and model answer. `fast_base_url` is user-only: a task can change the quick-reply model's name but never where requests go. When the second service is Google's Gemini API, any model named for it (`gemini-…` with no `google/` in front) goes there, the planner's included, so tasks can run on a free Google key while OpenRouter keeps Jev and web searches; without the Google key such a model runs on OpenRouter as `google/gemini-…`.

### Updates

`tauri-plugin-updater` with a minisign public key in `tauri.conf.json`. Two commands, `update_check` and `update_install`, are called only from Settings buttons; no model tool reaches them, and installing is refused while a task runs. The endpoint is `releases/latest/download/latest.json`, which the release workflow builds (`scripts/make-latest-json.mjs`) from the signed installers. Local and CI test builds don't make update files: `createUpdaterArtifacts` is switched on only by `src-tauri/tauri.release.conf.json`, which the release workflow passes.

### Settings and first run

- **Welcome:** on first start in demo mode (no brain set up, `first_run_done` unset) Settings opens on a short welcome: paste an OpenRouter key or a free Google Gemini key and **Test** it (`test_key`: one tiny call, metered like any other), find a local Ollama (`detect_ollama`: `GET /api/tags`, picks the tested Qwen model if it's there), or stay in the demo; then pick a colour. Saving or skipping sets `first_run_done`, so it shows once.
- **Tabs and search:** each fieldset says which tab it's on (`data-tab`); the search box shows every section whose words, placeholders or options match, across tabs. The last tab is remembered per browser profile. A field that fails validation opens its tab before the browser points at it.

## Decisions on the blueprint's models

| Item | Decision | Why |
|---|---|---|
| Laya (421M local decision model) | Deferred | Free, but ~1.5 GB RAM through Python and 0.2–0.5 s per call on CPU (the blueprint's 25–65 ms is GPU/WebGPU). Safety gating must be fixed rules anyway, and routing already happens inside the planner call. Revisit "needs a screenshot?" and "task done?" decisions, benchmarked against the activity log. |
| Jev (cloud router) | Not used | A paid extra network hop (`typesafe/jev-router` on OpenRouter). |
| Qwen2.5-VL / Qwen3-VL (local) | Replaced by Qwen3.5-4B | Qwen2.5-VL can't call tools in Ollama. Qwen3.5-4B (Feb 2026) beats Qwen3-VL-4B at computer use at the same size (OSWorld-Verified 35.6 vs 26.2, ScreenSpot-Pro 60.3 vs 59.5) and has vision and tools in Ollama ≥ 0.17.6. Its hidden thinking is off (`think: false`): it costs 30–60 s per step on a laptop CPU. |
| Other local candidates (Oct 2026) | Not default | Gemma 4 E4B: about twice the RAM, no GUI-agent results. Holo3.1-4B (a computer-use fine-tune of Qwen3.5-4B): better at clicking, but its vision file doesn't load in Ollama; try it through LM Studio. Snapdragon NPU runtimes (Nexa/GenieX, Foundry Local): free up the CPU, but tool calls with images are unproven there; next experiment. |
| Local vision model on Snapdragon | Optional, not default | Ollama runs CPU-only on Adreno today; a 4–8 GB vision model at full CPU breaks the headroom goal. Cloud default: ~$0.005 per task. |
| Tauri 3 | Not used | Still alpha; built on Tauri 2.12 stable. |

## Data flow for one click

1. The planner streams "I'll press Save." into the bubble.
2. It emits `click_element {id: 7}`.
3. The host locates element 7 through UIA, converts it to logical pixels, and sends `duck:move`.
4. The duck plans a path and walks or flies beside the button, then answers `duck_arrived`.
5. Tier 2 applies: the countdown card shows with Cancel. The timer runs in Rust; the UI can cancel early.
6. `duck:act peck` plays, and UIA invokes the button (or the host clicks its centre with the overlay forced click-through).
7. The result goes back to the model, and the activity log gets the gate decision and the outcome.
