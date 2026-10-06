# Waddle v0.3: the production pass

## Where things stand

- **v0.2 is done and merged** (milestones 0.1.9 → 0.2.0: routing, research, memory, the selection hotkey, Gmail, Calendar, Contacts, nudges, the morning brief, the Chrome extension, files and Drive, and the assistant bench). The v0.2 plan is in this file's git history.
- At the start of the pass, 169 Rust tests and the frontend and extension tests passed; at 0.2.9 it's 203 Rust tests and 71 frontend tests. Nothing has run on the Surface yet.
- **This pass:** nine releases, 0.2.1 → 0.2.9, that revise, adapt, enhance, polish and define Waddle. You test **0.2.9** on the Surface; the fixes from that test become **0.3.0**.
- **Status (0.2.9):** all nine releases are built. What each one changed is in [CHANGELOG.md](../CHANGELOG.md). Where the build differs from the plan below, the plan says so in *(as built: …)*.

## What the assessment found

**Corrections** (things that are wrong or fragile today)
- **Two Waddles at once.** Autostart plus opening it by hand starts a second copy: two ducks, two Chrome links and double nudges. There's no single-instance guard.
- **One hiccup fails a task.** A rate limit (new OpenRouter accounts get 20 requests a minute per model), a 502 or a dropped connection ends the task with "I couldn't reach my brain". A stream that stops sending hangs until the 5-minute task timeout.
- **Long replies get cut off silently.** The planner's 1,024-token reply cap can cut a long email, document or file in the middle of a tool call. The broken arguments then reach the tool.
- **Rotating, docking or changing the display breaks the overlay.** It's placed once at start-up. On a Surface, rotating to portrait, plugging in a monitor or changing the scale leaves the duck walking on the old screen size.
- **Unsafe saves.** Settings, facts, reminders, nudges, the style note and skills are written in place. A crash or power cut mid-write can lose them, and an unreadable `settings.json` quietly resets everything to defaults. The nudge state is rewritten every 15 seconds even when nothing changed.
- **An expired Google sign-in fails silently.** If the sign-in is revoked or expires, the watcher logs a warning every 90 seconds forever and Settings still says "Connected".
- **Shortcut tiers depend on spelling.** `alt+f4` needs a click, but `f4+alt` or `control+w` only get the 2 s notice. Key names aren't put in a standard form first.
- **Research files overwrite each other.** Asking the same question twice replaces the earlier `.md` file.

**Efficiency**
- Every step re-sends every earlier tool result. A 64 KB file read or command output is billed again at each later step, so long tasks get slow and costly. Screenshots are already trimmed; text isn't.
- Chat replies wait for the whole answer before showing anything.
- The conversation is forgotten when Waddle restarts.
- Nothing adds up what Waddle spends. You set a $5-a-month target, but nothing shows the running total or stops at a limit.

**Gaps compared with the best desktop agents in 2026**
- **No way to add tools.** The Model Context Protocol (MCP) is now the standard way to connect an agent to other apps and services. Waddle can't use MCP servers.
- **No scheduled work.** Reminders chime, but they can't run a task ("every weekday at 8:45, tell me what's on today").
- **No spoken replies** (deferred from v0.2), even though Windows 11 has free, offline neural voices.
- **No conversation history.** Once the bubble fades, an answer is gone. There's also no way to recall what you typed earlier.
- **One monitor only.** The duck lives on the primary monitor even when you work on another one.
- **No first-run guide.** Setup is spread over eleven sections of a long Settings page.
- **No visual effects** (deferred from v0.2), and error messages are written for developers.

## Releases

Each release is tested, committed and pushed with its version bump. They're delivered in order.

### 0.2.1 Revise: reliability
- **Single instance.** A second launch opens the running Waddle's chat box instead of starting a second duck. The Chrome relay (`waddle.exe chrome-extension://…`) is unaffected.
- **Model calls survive hiccups.**
  - Rate limits, 5xx errors, dropped connections and cut streams are retried up to twice on the **same model**. Waits are 1 s then 3 s, or what `Retry-After` asks for, up to 8 s.
  - A retry happens only if nothing has been shown to the user yet. Escalating to a stronger model still never happens.
  - A stream that goes quiet for 60 s is treated as dropped.
  - Errors read like "OpenRouter is rate-limiting this model; try again in a minute", not a raw HTTP dump.
- **No more cut-off replies.**
  - The planner's reply cap goes from 1,024 to 4,096 tokens. The cap only limits output, so it costs nothing extra unless used.
  - A reply that still hits the cap (`finish_reason: length`) is never run as a broken tool call. The model gets one nudge to write shorter content or split it.
- **Follows display changes.** The overlay re-places itself within about 2 s when the work area, resolution, scale or orientation changes (rotating the Surface, docking, moving the taskbar).
- **Crash-safe saves.**
  - Every store writes to a temporary file and then renames it.
  - An unreadable settings file is kept as `settings.json.bad` and Waddle says so, instead of silently resetting.
  - The nudge state is saved only when it changes.
- **Expired Google sign-in.** When Google refuses the saved sign-in (`invalid_grant`), Waddle:
  - stops polling
  - says once "Google signed me out; reconnect in Settings"
  - shows "Sign-in expired" in Settings.
- **Shortcut tiers.** Key names are put in a standard form before classifying: modifier order, `control`/`ctrl`, `del`/`delete`, `win`/`meta`/`super`/`cmd`.
- **Research file names** get `-2`, `-3`… instead of overwriting.

### 0.2.2 Revise: leaner, faster, cheaper
- **Context budget.**
  - Once a task's conversation passes about 48,000 characters, earlier tool results longer than 1,500 characters are cut down to their start plus "[trimmed; read it again if you need it]".
  - This happens all at once, like screenshots, so a local model's prompt cache misses only once.
- **Chat replies stream.** The first words show in about half a second. The "[task]" hand-off is still detected from the first few tokens.
- **Conversation survives restarts.** The last 10 exchanges are kept in `memory.json`, cleared after 12 hours of silence or by **Forget conversation**.
- **Spending meter and monthly budget.**
  - Every paid call is recorded in a daily ledger by role: tasks, chat, research, quick replies, decisions, briefs and style learning. OpenRouter reports what each call costs.
  - Settings shows today, this month and a 30-day breakdown.
  - A monthly budget (default **$5**) gives one warning at 80%. At 100% Waddle pauses paid calls ("I've reached this month's $5 budget; raise it in Settings"). Stop, reminders and the screen still work.
- **Battery-aware.** On battery, the window sampler, hit test and ambient brain slow down (on Windows, from `GetSystemPowerStatus`).

### 0.2.3 Adapt: multiple monitors
- **Waddle follows you.** When the focused window has been on another monitor for 2 s, the overlay moves to that monitor's work area and the duck flies in from the edge. During a task, the monitor stays fixed until the task ends.
- Screenshots, window lists, element coordinates and clicks use the duck's current monitor, including mixed scaling (for example, a 200% Surface screen next to a 100% external one).
- The model is told which monitor it sees and its size.
- **Settings → Character:** "Follow me across monitors", or stay on the primary monitor.

### 0.2.4 Enhance: conversation panel
- **History drawer.** A ▴ button on the chat box, or **Ctrl+Alt+H**, opens:
  - the last 50 messages: yours, Waddle's answers, research summaries and nudges
  - safe Markdown (paragraphs, lists, bold, code, links that open in the browser) and a copy button on each message
  - a **Full answer** button that still works
- The history is kept across restarts. **Forget conversation** clears it.
- **Chat box.**
  - ↑ and ↓ recall what you sent before.
  - Long or pasted text grows the box to several lines. Shift+Enter adds a new line.
- **Long answers.** The bubble shows the first three lines, then **More…**, which opens the drawer at that answer.

### 0.2.5 Enhance: spoken replies
- **Windows' built-in neural voices** (WinRT `SpeechSynthesizer`): free, offline, on ARM64.
  - Waddle can speak final answers, chat replies and nudges. Each kind has its own switch.
  - Settings has a voice picker, speed and a **Test voice** button.
- **When it stays quiet:**
  - Speaking stops when you say or press stop, start talking, or open the chat box.
  - It stays silent while you're in full screen or in a meeting (your calendar says so).
- **Talk mode (optional).** After a spoken answer to a spoken question, the microphone stays open for 6 seconds for a follow-up.
- On Linux (development only), `spd-say` is used when it's installed.

### 0.2.6 Enhance: MCP tools (connect anything)
- **Settings → Tools.** Add an MCP server by name, command, arguments and environment (secrets go to the keychain), with **Test** and an on/off switch per server.
  - Waddle lists the server's tools and offers them to the planner as `mcp.<server>.<tool>`.
- **Safety.**
  - Tools the server marks read-only are tier 1. Everything else is tier 3, or tier 2 if you mark the server "trusted".
  - Results are untrusted data.
  - Servers start only when needed, stop after 10 idle minutes and are killed on quit.
  - The model can never add or start a server.
- **Protocol.** Standard input/output transport with the 2025-11-25 handshake, which most servers speak today. Streamable HTTP and the stateless 2026-07-28 revision come later.
- **Testing.** A small fake server checks listing, calling, errors, timeouts, tiers and halting.

### 0.2.7 Enhance: routines
- **Scheduled tasks.**
  - "Every weekday at 8:45, tell me what's on today and anything urgent in my inbox"
  - "every Friday at 4, list the meetings I had this week"
  - "at 6 tonight, check whether the parcel email arrived"
- Created with a `routine` tool (tier 2) or in Settings; listed, paused and deleted in Settings.
- **How a routine runs.**
  - In the background without touching the screen: Google, files, Chrome page reading, research and MCP tools only.
  - Any tier 2 or 3 action waits for your click, because nobody is watching a countdown.
  - The result arrives as a nudge and goes into the history drawer.
- **Missed runs.**
  - If Waddle is busy, the routine runs straight after.
  - If the computer was off, the run is marked missed, like reminders.
  - Routines respect the budget.

### 0.2.8 Polish: first run, settings, effects
- **First-run welcome.**
  1. Choose a brain: paste an OpenRouter key and Waddle tests it live, detect Ollama, or stay in the demo.
  2. Optional: connect Google and set up Chrome, each with a status tick.
  3. Pick the duck's colour.
- **Settings reorganised into tabs:** Brain, Assistant (Google, Chrome, files), Nudges & routines, Tools, Voice, Memory & privacy, Safety, Character, Diagnostics. There's a search box, and spending sits at the top. *(as built: seven tabs; Tools sits in Assistant and Safety in Brain, beside spending.)*
- **Effects layer**, drawn from the duck's palette and turned off with reduced motion:
  - dust when it lands
  - "Zzz" while it naps
  - a sparkle when a task finishes
  - a sweat drop on errors
  - "?" when it asks
  - a heart when you say thanks
- **Errors in plain words.** For example, "OpenRouter says the key has no credit left: add some at openrouter.ai/credits". The common failures each get a next step.
- **Accessibility.** Keyboard focus rings and Tab order on cards and the drawer, Enter/Esc on approval cards, and high-contrast support. *(as built: Esc denies; deliberately no Enter shortcut to approve, so typing elsewhere can't approve anything.)*

### 0.2.9 Define: release candidate
- **Regression.**
  - All unit and integration tests.
  - The assistant bench, router set and importance set (small spend; see Budget).
  - A live container run of every checklist item that can run here.
- **Self-test covers the new parts:** speech, monitors, MCP servers, budget and routines.
- **Docs:**
  - the README rewritten around what Waddle does
  - ARCHITECTURE and MODELS updated (a fresh look at the cheapest planner)
  - a new CHANGELOG
  - **on-device checklist v2:** the 27 checks plus the new features, grouped, each with what you should see
- **Installer** `Waddle_0.2.9_arm64-setup.exe` → **you test on the Surface.** *(as built: also a tag-triggered release workflow that builds ARM64 and x64 installers into a draft GitHub release, plus LICENSE, CONTRIBUTING and SECURITY for open-sourcing.)*

### 0.3.0
- Fixes from your 0.2.9 test, docs brought up to date, and the final installer.

## Needs you (can't be done from this container)
- **On-device testing** of 0.2.9 with the checklist.
- A **code-signing certificate** (removes the SmartScreen warning).
- A **Chrome Web Store** developer account (no more Developer mode).
- **Auto-update:** an updater signing key stored as a GitHub secret, then Waddle can update itself from Releases.
- **Rotate the OpenRouter key** used for testing, and press **Connect** again in Settings → Google (for the Drive permission).

## After 0.3
- macOS (accessibility and screen-recording prompts, Apple Silicon build).
- Foundry Local NPU mode, and a fine-tuned local planner ([TRAINING.md](TRAINING.md)).
- The "Pictionary" defence: untrusted text shown to the model as an image.
- PTY terminal and a real OS sandbox for commands.
- The other seven characters and a white-label asset pack loader.

## Working rules for this pass
- **Before each push:**
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `npm test`
  - `npm run typecheck`
  - `npm run build`
  - `cargo xwin check --target aarch64-pc-windows-msvc -p waddle` for the Windows-only code
- **Budget.** Model testing for the whole pass stays under **$0.40**, on top of v0.2's $0.30. Running Waddle stays under $5 a month.
- **Safety rules don't move:**
  - tiers are fixed rules
  - approvals only come from a click
  - untrusted text is never instructions
  - no escalation to a stronger model
  - keys never in files or logs
- **Can't be checked here,** so they go on the on-device checklist:
  - Windows voices
  - real multi-monitor layouts
  - the single-instance handover on Windows
  - battery state
  - the first-run flow on a fresh install
