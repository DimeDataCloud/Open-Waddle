# Waddle 0.3.0: the blueprint

0.3.0 is the release that follows the first weeks of real use. It is built from
three batches of the user's own task traces (the opt-in recorder in
Settings → Memory & privacy), the open items from the 0.2 pass, the Gemini work
(PR #15), and current research on desktop agents. Each workstream below says
what the evidence showed, what changes, and how it's verified.

## What the traces showed

There were 34 recorded tasks in the third batch (plus 22 rows in the first two).
Every number below comes from those traces.

| Finding | Evidence | Workstream |
|---|---|---|
| Fake "I did it" replies | 9 of 14 tasks on the old planner model answered "I've opened YouTube…" without any tool call. The chat lane did the same. Fixed in PR #16; 0.3 keeps the guards and adds learning from it. | E |
| Accidental stops | 3 tasks in a row stopped before the first model reply came back. This happened twice, an hour apart. The most likely cause is Esc pressed to close the chat box: Waddle registers Esc as the stop key while it works. | A |
| Waiting on countdowns | Every key press took 2.1 s and every click 3 to 4.6 s. Nearly all of that is the tier-2 countdown (2 s) plus the walk. A split-screen request made 13 key presses. | B |
| Too many steps | One "PRs on the right, music on the left" task took 20 steps (80 s of model time). About 7 would do: reads after every click, a tab list before every navigation, and window snapping by keyboard shortcuts plus screenshots to check. | B, C |
| Slow model steps | Median model step: 3.2 s on the default planner, 2.0 s on Qwen3-VL-8B. Steps cost more time than anything else, so removing steps is the biggest speed-up. | C |
| Misheard app names | Voice gave "Clawed" for Claude. `open_app` failed twice, and then a shell fallback sat on an approval card for 3 minutes. | D |
| Requests for the duck itself | "Fly around the screen" and "make a mess" went to the planner, which has no tool for the duck's own body. It carried on with an old YouTube task instead. | F |
| Good runs to learn from | 6 tasks were rated 👍, among them YouTube Music playlists, split screen and GitHub pull requests. Their tool sequences are reusable recipes. | G |

## Research that shaped it

- **Leaderboards measure the whole agent, not the model.** OSWorld-Verified rankings reflect the full stack: grounding, planning and error recovery ([OSWorld-Verified](https://yutori.com/leaderboards/osworld-verified.md), [a16z](https://a16z.com/can-agents-use-a-computer-yet-weve-got-the-data/)). That matches what we found in 0.2: the harness mattered more than the model.
- **Hybrid GUI + API actions and multi-action steps** ([UFO², Microsoft Research](https://arxiv.org/html/2504.14603v2)). Use native APIs where they exist instead of clicking, and let one model call plan several actions, checking each one's preconditions before it runs. 0.3 does both: window and browser APIs, and batched calls that stop at the first failure.
- **Workflow memory** ([Agent Workflow Memory, ICML 2025](https://arxiv.org/abs/2409.07429)). Reusing routines that worked raised success by 24.6% (Mind2Web) and 51.1% (WebArena) relative, with fewer steps. 0.3 learns recipes from tasks that worked and offers the closest one on similar requests.
- **Gemini computer use** ([Google, June 2026](https://vpsranking.com/news/ai/ai-2026-06-24-google-gemini-35-flash-computer-use/)). Computer use is now built into Gemini 3.5 Flash. Waddle already routes the quick lanes to Google's API (PR #15). 0.3 benchmarks Gemini models on our own harness before changing any default.

## Workstreams

### A. No more accidental stops
- **Esc closes an open chat box (or the history drawer) first; the next Esc stops the task.** The overlay tells the backend when either panel is open. The Esc hotkey closes the panel instead of halting.
- **"Try again" on a stopped task.** It runs the same request again in one click, so an accidental stop costs a second.
- The stop message already names what stopped it (PR #16).
- *Verify:* unit tests for the Esc decision; frontend test for the button.

### B. Fewer waits
- **Safe keys run straight away (tier 1).** These are window snapping (Win+arrows), switching windows and tabs, new tab or window, the address bar, scrolling keys, media keys and zoom. Keys that can close, delete or submit keep their tier.
- **A window tool:** `arrange_window` focuses, snaps left or right, maximizes, minimizes, restores, or moves a window to another monitor through the OS API, in one tier-1 step. On Windows it uses SetWindowPos with the monitor's work area. On Linux it's best effort through `wmctrl` or `xdotool`.
- *Verify:* safety tests for every new tier-1 key and every key that stays tier 2 or 3. Unit tests for the snap geometry. A live test on Xvfb with two monitors.

### C. Fewer steps
- **Read in the same step.** `browser_navigate`, `browser_click` and `browser_tabs open` take `read: "text" | "elements"` and return the page read with the result, which saves one model call each time.
- **A new window in one call:** `browser_tabs open` with `new_window: true` (extension 0.3.0).
- **Site recipes in the tool description.** These are direct search addresses for YouTube, YouTube Music, GitHub, Google, Maps, Wikipedia and Amazon, plus a channel's videos page. "Search YouTube for X" becomes one call.
- **Batches stop at the first failure.** The prompt asks for several calls in one reply when the next steps are clear. If one fails, the rest are skipped and the model is told why.
- *Verify:* agent-flow tests with a scripted model; extension build test; tool-description tests.

### D. Understanding speech
- **App names are matched by sound and spelling.** If no app matches exactly, `open_app` tries the installed apps (Start menu shortcuts) by similar spelling and sound, and opens a clear match ("Clawed" → Claude). Otherwise its error names the closest ones, so the model asks the user instead of reaching for the shell.
- **Voice text is tidied** before it's routed: wrapping quotes and leading fillers ("uh", "um") are removed. The history keeps what was said.
- *Verify:* unit tests built from real misrecognitions (scrubbed).

### E. Honest replies, kept
- The PR #16 guards stay: the claim check, the router bypass for plain requests, and the sentence gate in the chat lane.
- **In 0.3 they also feed training:** a reply that claims an action without a tool is exported as a *negative* example.

### F. The duck's own body
- **A `duck` tool (tier 1):** fly around the screen, come over to the pointer, nap, wake up, dance or hide in a corner. All of it is played by the overlay that already knows these moves. "Fly around the screen" now does exactly that.
- *Verify:* tool tests; frontend tests for the move sequences.

### G. Learning from experience (training without a GPU)
- **Workflow memory** (`experience.rs`). When a task ends well (rated 👍, or done with tools and no false claims), its tool sequence is condensed into a short recipe. Addresses and app names are kept; typed text and email bodies are dropped. When a later request is similar enough, the closest recipe goes into the planner prompt as a hint ("a way that worked before; the screen may differ"). A 👎 removes the recipe that task made. It's capped, stored on the computer, and cleared by Forget conversation.
- **Training export v2.**
  - Positive examples skip replies that claim an action without a tool.
  - A **KTO file** (thumbs up/down as desirable/undesirable labels) is written next to the chat-format file.
  - A **scrubbed** option masks email addresses, phone numbers and user folders.
  - Repeated conversation prefixes are dropped.
  - Pipeline steps are in [TRAINING.md](TRAINING.md).
- **Model choice on data.** The Gemini models (3.5 Flash, 3.5 Flash-Lite, 3.8 Flash) run on the assistant bench and the chat-honesty test. Defaults change only when a model wins on both. Results are in [MODELS.md](MODELS.md).
- *Verify:* unit tests for recipe extraction, matching and privacy (no typed text or email content in recipes); export tests.

### H. Release
- Version 0.3.0 everywhere (app, extension 0.3.0). CHANGELOG, README (new controls and features, checklist additions), ARCHITECTURE, ROADMAP.
- All checks: clippy with warnings as errors, Rust tests, frontend typecheck, tests and build, the extension build, and Windows cross-checks for arm64 and x64.
- A pull request, merged once CI is green. The tag and the published release stay with the maintainer, who pushes the tag.

## Not in 0.3
- Fine-tuning a model. There's no GPU here, and there aren't enough rated traces yet: 6 good tasks against the ~300–1,000 the plan in [TRAINING.md](TRAINING.md) calls for. The export and the KTO file make the first run a one-command job when there are.
- Gemini's native computer-use tool (screenshot in, click out). Waddle's own tools are more precise where an API exists. We'll revisit once the bench shows how Gemini does with them.
- Code signing waits on SignPath; release.yml changes when it's approved.
