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
- **Conversation memory:** the last 10 exchanges carry over between tasks.
- **Local prompt cache:** within a task the conversation is append-only, so a local server (Ollama/llama.cpp) reuses its cache and only processes new tokens each step. Local models keep up to 2–3 screenshots before older ones are dropped in one go (hosted APIs keep 1, since every image is billed on every call). Opening the chat box warms a local model: it loads and reads the system prompt, tools and conversation so far while the user types, so the first step only reads their message (36.6 s → 6.8 s here on a 4-core CPU). The warm-up asks Ollama for thinking on, because the empty think block that `think: false` appends ends the prompt past Qwen3.5's last cache checkpoint.

### Assistant tools

| Tool | Tier | What happens |
|---|---|---|
| `point_at` | 0 | The duck walks to the spot and the overlay draws a pulsing ring with a label for 8 s. The ring never takes clicks. Prompted for "where is…" and "how do I…" questions. |
| `scroll` | 1 | Mouse-wheel notches at a point or under the cursor, one notch at a time so apps that smooth scrolling see real scrolling. Counts as blind input, so it needs a look first. |
| `drag` | 2 | Press, glide in 20 steps, release, so apps register a drag (files, sliders, windows). |
| `read_clipboard` / `copy_to_clipboard` | 1 / 2 | `arboard`. Clipboard text is wrapped as untrusted, since a web page can put text there. |
| `reminder` | 1 | add / list / cancel, stored in `reminders.json` in the app data folder (`reminders.rs`). A thread checks every 5 s and emits `reminder`; the overlay chimes and pins the bubble for a minute. Reminders that came due while the app was closed pop up at start, marked as missed. |

- **Text tool calls:** Qwen sometimes writes a call as text (`point_at(x=920, y=240)`, `</tool_call>`) instead of making it. `writes_tool_call` spots an offered tool name followed by `(` or `{`, and the model gets one nudge to make the real call.
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
