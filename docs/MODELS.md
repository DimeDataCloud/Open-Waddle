# Choosing Waddle's brain (October 2026)

Short version:
- **New cloud default: `openai/gpt-6-luna`.**
  - It did the right thing in 41 of 42 benchmark runs, against 38 for Qwen3-VL-8B.
  - It went 3 for 3 on live tasks on real websites, against 2 for 3 for Qwen.
  - It costs about half as much per task (~$0.001).
- **Fallback: `qwen/qwen3-vl-8b-instruct`**, the previous default, which is still solid. Set it as the model in Settings to switch back.
- **Before each task on OpenRouter, TypeSafe's Jev decision model** checks whether the screen is needed at all, and skips the screenshot when it isn't (see below).
- **The harness mattered far more than the model.** The same model scored 27% before.
- **v0.2 picks (October 4, 2026), all confirmed by testing:**
  - **Planner:** `openai/gpt-6-luna`. 12/12 on the new assistant bench in both passes, at about $0.0005 a task. That's the cheapest of the models that passed everything.
  - **Chat and research:** `google/gemini-2.5-flash-lite`. Replies take 1–1.6 s, and a cited research summary takes 4.5 s.
  - **Decisions** (routing, the screen check, mail importance): Jev 1.13, at about $0.00002 each.
- **0.3.0 (October 6, 2026): Gemini on our own harness.** `gemini-3.5-flash-lite` passed every assistant job (12/12) in **2.9 s a task, about twice as fast as `openai/gpt-6-luna` (5.6 s)**, and did at least as well on the screen tasks. Through OpenRouter it costs about 4× as much; **on a free Google key it costs nothing.** So:
  - with a free Gemini key: run tasks on it (Settings → Brain → Quick replies run on **Google Gemini API** → tick **Run tasks on Gemini too**, or pick Google Gemini in the welcome);
  - without one: `openai/gpt-6-luna` stays the default, the cheapest model that passes everything.
  - Details are in [Gemini on Waddle's harness](#gemini-on-waddles-harness-030-october-6-2026).
- Two moves are worth making next:
  - try the newer, cheaper Qwen Flash models (your OpenRouter guardrail blocks them for now)
  - fine-tune a 4B model for the local, free brain ([TRAINING.md](TRAINING.md))

## What Waddle needs from a model

Waddle sends one screenshot plus a small tool list, and gets back one short sentence and one tool call, about 2–3 times per task. The model must:

1. **Ground clicks:** turn "the Lo-fi Study Mix playlist" into the right pixel.
2. **Call tools reliably:** OpenAI-style tool calls, not prose about calling them.
3. **Use judgement:** act in the app that's open, and don't click what the task doesn't need.
4. **Be quick and cheap:** every step makes the user wait, and every image is billed.

Long-horizon planning (OSWorld-style 50-step tasks) matters less than the first three.

## The benchmark (`bench/`)

We test models on our own harness, with the same prompt, tools and tool parsing the app uses, instead of trusting leaderboard numbers.

- **15 tasks on realistic screens.** These are HTML pages rendered in Google Chrome at the Surface's logical resolution, 1440×960:
  - a video site
  - webmail
  - Windows Settings
  - a word processor
  - File Explorer
  - a Notepad save dialog
  - a web form
- **Plus three tasks without a screen:** a file task, an accessibility-list task, and a "just answer, don't click" task.
- **Each task checks what Waddle actually did,** for example "a click inside the Lo-fi Study Mix card" or "typed into the search box, then Enter".

Run it:

```bash
OPENROUTER_API_KEY=… WADDLE_BENCH_MODELS=qwen/qwen3-vl-8b-instruct,google/gemini-3.1-flash-lite \
  WADDLE_BENCH_REPEAT=3 cargo test -p waddle-core --test bench -- --ignored --nocapture
# local: WADDLE_BENCH_PROVIDER=ollama WADDLE_BENCH_MODELS=qwen3.5:4b …
# new screens: add a page under bench/pages with data-t="name" on targets, run node bench/capture.mjs
```

Results land in `bench/results/*.json`, with every action each model took.

**Limits:**
- The fake desktop doesn't change after a click. A model that "checks its work" sees the same screen and may click again, so multi-step tasks score a little low.
- The pages were built to look like real apps, but they aren't the real apps.

## Results

3 runs of each task (45 runs per model):

| Model (OpenRouter) | Harness | Passed | Time per task | Cost per task |
|---|---|---|---|---|
| qwen/qwen3-vl-8b-instruct | old (no look first) | 4/15 (27%), 1 run | 3.5 s | $0.0006 |
| qwen/qwen3-vl-8b-instruct | look first | 38/45 (84%) | 8.1 s | $0.0009 |
| **qwen/qwen3-vl-8b-instruct** | **current** | **42/45 (93%)** | **6.3 s** | **$0.0009** |
| google/gemini-2.5-flash-lite | look first | 27/45 (60%) | 5.7 s | $0.0006 |
| google/gemini-2.5-flash-lite | current | 30/45 (67%) | 4.6 s | $0.0005 |

**Where they fail:**
- **Qwen3-VL-8B:**
  - Only one failure left: asked to create a file while a Notepad "save changes?" dialog was open, it clicks Cancel on the dialog first (harmless, but not asked for).
  - Before the element-name check, it named "Don't save" but sent the id of "Save" every time.
- **Gemini 2.5 Flash Lite:**
  - It misses small targets by 15–40 px: the Night light toggle, the Delete icon, sidebar items.
  - It sometimes opens Notepad to "write" a file instead of using the file tool.
  - It's fine for its current job, quick chat replies while a task runs, but not for acting.

### What changed the score (the harness)

| Change | Effect |
|---|---|
| **Look first.** Every task starts with the window list and a screenshot attached to the request (`look_first`). | 27% → 84%. Without it the model said "I can't play music" or "which app?", or clicked the screen's centre blind. This was the main cause of the "5 attempts and corrections" on 0.1.2. |
| **Name-checked `click_element`.** The model gives the element's name too, and the name wins when the id disagrees. | Accessibility task 0/3 → 3/3 |
| **No third identical click.** Re-clicking the same spot is refused with "try something else". | Stops loops where a model clicks a dead button until the step limit |
| **"The screenshot is context, not a to-do list"** (prompt) | Didn't fix the file-task over-click; kept because it's cheap and right |

## Newer models (tested October 4, 2026)

The suite now has 21 tasks: the 15 above plus 6 assistant tasks (point at something without clicking, scroll, drag a file, copy to the clipboard, set a reminder, and a second "show me where"). 2 runs per task.

- **Strict** means the task finished with the right actions.
- **Hit** forgives a model that did the right thing, then kept re-checking the test screen until the step limit. The screen is a still image, so a careful model sees "nothing happened" after a correct click.

| Model | Strict | Hit | Time per task | Cost per task | Notes |
|---|---|---|---|---|---|
| **openai/gpt-6-luna** | 15/42 | **41/42** | 11 s† | $0.0009† | **New default.** Right action almost every time, then re-checks the static screen. Pixel coordinates. New accounts: 20 requests/min per model |
| qwen/qwen3-vl-8b-instruct | 38/42 | 38/42 | 4.1 s | $0.0011 | Previous default. Misses: the known file-task over-click, one copy done by typing and pressing Ctrl+C |
| google/gemini-3.8-flash | 17/42 | 41/42 | 17 s† | $0.0135† | As accurate as Luna but 13× the price |
| qwen/qwen3.5-flash-02-23 | 35/42 | 38/42 | 11.1 s | $0.0008 | As accurate; slower, re-checks more |
| qwen/qwen3.7-flash | 14/42* | — | 29 s | $0.0005 | Answers in its own pixel space (a click at x=1714 on a 1440-wide screen); pixel mode didn't fix it (1/6). Thinks for a long time |
| qwen/qwen3.8-flash | 14/42* | — | 22 s | $0.0013 | Same coordinate problem as 3.7 |
| qwen/qwen3.8-27b | 18/42* | — | 15 s | $0.0024 | First clicks look right; then re-checks the static screen and re-clicks |
| google/gemini-3.1-flash-lite | 6/42* | — | 8 s | $0.0038 | First clicks right (Night light, Delete, Compose), then re-clicks to the step limit; pricier than Qwen |

**Quiet mode (0.1.9 default voice):** the same 21 tasks × 2 with openai/gpt-6-luna, now that the model acts without narrating and speaks only to finish, answer or ask: **25/42 strict, 42/42 hit, 7.1 s and $0.00064 per task** (3.6 steps on average). Faster and cheaper than the narrating run above, with fewer wasted re-checks.

\* First run, before the hit score and the concurrency cap. Rate limits and re-checks count as failures, so these numbers are a floor.
† Includes the wasted re-check steps on the static screen; on real pages, where the screen changes, Luna needs fewer steps (below).

### Live check on real websites (screens that change)

Real Google Chrome, the real app, the real model, one run each:
- "Search Wikipedia for rubber duck debugging"
- "Open the newest stories page" (Hacker News)
- "Use the site search to find the CSS grid layout guide and open it" (MDN)

| Model | Passed | Time per task | Cost per task |
|---|---|---|---|
| openai/gpt-6-luna | 3/3 | 24 s | $0.0010 |
| qwen/qwen3-vl-8b-instruct | 2/3 (answered without acting on Hacker News) | 16 s | $0.0021 |

## v0.2 bake-off: assistant tasks (October 4, 2026)

`crates/waddle-core/tests/assist.rs` runs 12 text-only jobs against a fake Google, a temporary folder and a fake Chrome extension, using the app's real prompt, tools and tiers. Each check looks at what happened, not just the reply. For example:
- **reply:** an email went to Ana, in her thread, and says Thursday
- **book:** an event with Sam at 14:00 that has a Meet link
- **signup:** typed into field e1 and clicked e3
- **delete_guard:** asked before deleting, and kept the file when the user said no

Running out of steps or hitting a provider error counts as a failure.

The 12 jobs:

| Area | Jobs |
|---|---|
| Calendar | next meeting, booking with a contact and a Meet link |
| Mail | important unread mail, threaded reply, archiving |
| Files | rent from a PDF, a figure from an XLSX, making a spreadsheet, a refused delete |
| Chrome | filling a form, a web search |
| Memory | remembering a fact |

| Model | Passed | Time per task | Cost per task | Notes |
|---|---|---|---|---|
| **openai/gpt-6-luna** | **36/36** (3 passes) | 5.7 s (when not rate-limited) | **$0.0005** | **Keep as the default.** Short, correct answers. |
| openai/gpt-6-luna-pro | 12/12 | 10–14 s (partly rate-limit waits) | $0.0015 | As accurate, 3× the cost and slower. Lists every unread email when asked for important ones. |
| xiaomi/mimo-v2.6-flash | 12/12 | 6.1 s | $0.0019 | Accurate and quick, 4× the cost. Writes Markdown, which the bubble strips. **The best alternative.** |
| qwen/qwen3-vl-8b-instruct | 9/12 | 3.8 s | $0.0019 | Saved a draft instead of sending, missed the urgent email, and ran out of steps on the form. |

New accounts are limited to 20 requests a minute per model, so the bench retries after a 429. Times above leave those waits out. The whole bake-off, with reruns, cost about $0.2.

**Release candidate check (0.2.9, October 6, 2026):** `openai/gpt-6-luna` passed all 12 jobs again, at about 5 s and $0.0005 a task, after one fix: a Gmail search for "newsletter" found nothing (Gmail matches words literally), and the model gave up on *archiving*. An empty search now tells it to look through the inbox and judge by sender and subject; *archiving* then passed 4 of 4. Runs that hit the 20-a-minute limit were re-run. Jev's routing, screen check and mail importance (29 of 30) were unchanged.

**Chat and research models (one pass each):**
- `google/gemini-2.5-flash-lite`:
  - small talk and a knowledge question: 1.0–1.6 s
  - a current-facts question with web search: 3.2 s
  - a research summary: first words in 2.0 s, done in 4.5 s
- `inception/mercury-2.5`: 4.9 s for a "thanks", and it handed small talk off to a task. Not used.

## Google's Gemini API directly (October 2026)

Waddle can use Google's own endpoint (`https://generativelanguage.googleapis.com/v1beta/openai/`, OpenAI-compatible) for the planner, for the quick-reply model, or for both next to OpenRouter (Settings → Brain → **Quick replies run on**).

| | Model name on Google | Name on OpenRouter | Google free tier | OpenRouter price per M tokens |
|---|---|---|---|---|
| Newest Flash | `gemini-3.8-flash` | `google/gemini-3.8-flash` | free | $0.75 in / $3.75 out (doubles on 1 Jan 2027) |
| Newest Flash-Lite | `gemini-3.5-flash-lite` | `google/gemini-3.5-flash-lite` | free | $0.30 in / $2.50 out |
| Old default | (restricted to existing projects) | `google/gemini-2.5-flash-lite` | n/a | $0.10 in / $0.40 out |

- **Privacy:** on the free tier Google says content is used to improve its products. Waddle sends screen text, files and email text, so treat the free tier as a way to try things, and turn on billing for private use. OpenRouter requests ask providers not to train on prompts.
- **Rate limits** aren't published; Google shows them per account in AI Studio.
- **What differs:** Google's endpoint rejects OpenRouter's extra request fields, so Waddle leaves them out for it (hidden-thinking effort and the web-search plugin). Searches for current facts and research therefore go through the planner's service even when quick replies are on Google.
- **Metering:** only OpenRouter reports a dollar cost, so calls to Google show as calls in the spending meter without a cost, even on a paid Google plan.
- **Benchmarked in 0.3.0:** both pass every assistant job; see below.

### Gemini on Waddle's harness (0.3.0, October 6, 2026)

Through OpenRouter, with the 0.3.0 prompt and tools (arrange_window, read-in-the-same-step, search addresses, the duck tool). Runs that hit OpenRouter's 20-requests-a-minute limit for new accounts were re-run one at a time; times leave those waits out.

| Model | Assistant jobs (12) | Time per task | Cost per task (OpenRouter) | Screen tasks (21): passed / right target | Time per screen task |
|---|---|---|---|---|---|
| `openai/gpt-6-luna` (default) | **12/12** | 5.6 s | **$0.0005** | 14 / 21 | 7.7 s |
| `google/gemini-3.5-flash-lite` | **12/12** | **2.9 s** | $0.0018 | **16 / 21** | **5.0 s** |
| `google/gemini-3.8-flash` | **12/12** | 8.0 s | $0.0082 | not run | |

- Every model hit the right target on every screen task. The "passed" count is lower because the fake desktop doesn't change after a click, so a model that checks its work clicks again until it runs out of steps (see [Limits](#the-benchmark-bench)).
- **Reading:** Flash-Lite is the quickest model that passes everything, and quick is what matters most once a task is right: a model step is most of the wait (the median step in real use was 3.2 s on the default). 3.8 Flash is as accurate but slower and costlier, so it isn't worth it for Waddle's short tasks.
- **On a free Google key** Flash-Lite costs nothing. Keep OpenRouter's key too: routing (Jev), the screen check and web searches stay there, and if the Google key is missing or rate-limited out, the same model runs through OpenRouter instead (`google/gemini-3.5-flash-lite`). With only a Google key, everything runs on Google (no quick chat lane: every message goes to the planner).
- **Not yet checked against Google's own endpoint** (no key in the test environment). Use the **Test** button in Settings after pasting a key.
- These runs cost about $0.21 in all.

## Jev and Laya (decision models)

These are "System One" models: one quick pass that returns a yes/no probability, a choice or a score, not text. They suit small decisions around the main model, not acting.

| | Jev 1.13 (TypeSafe) | Laya (Convai, Apache-2.0) |
|---|---|---|
| Where it runs | OpenRouter, `POST /api/alpha/decisions` (works with Waddle's key) | Local ONNX (`onnxruntime`), 421M parameters, ~1.2 GB int8 |
| Speed | 0.3 s median, 0.4 s at p90 (measured) | 150–460 ms on CPU (model card); x86_64 builds only so far |
| Cost | ~$0.000015 per decision (input tokens only) | free, but ~1.2 GB of RAM while loaded |
| Tools or images | no | no |

**In use now: "does this task need the screen?"** (`smart_look`, on by default with OpenRouter).
- Jev scored 28/32 on sample requests, and every miss was on the safe side (it looked when it didn't need to).
- Below 0.3 the opening screenshot is skipped. That would have skipped 8 of the 12 tasks that didn't need the screen (reminders, files, maths, writing), with no wrong skips. Every task that needed the screen scored 0.5 or more.
- Any error, or an answer slower than 2 s, means "look".
- With Google connected, the question says mail, calendar and contacts are reachable without the screen, and the skip bar rises to 0.6. Live: "what's my next meeting?" 0.23, "any important unread email?" 0.51, "reply to Ana…" 0.48, "find 45 minutes with Sam…" 0.33, against "summarise this email" 0.83 and "click the blue button" 0.99. That took "what's my next meeting?" from 5.2 s to 3.3 s end to end.

**Also in use: ambient behaviour and the chat lane.**

- **Ambient behaviour:** Jev picks the duck's next idle behaviour from the front app's name, full screen or not, and idle time. In the app it chose:
  - perch while Chrome was in use
  - give space when Chrome went full screen
  - nap after a minute with no mouse movement

  Each decision took 0.26–0.6 s, and a busy day costs a few cents at most.
- **Router (0.1.9):** one Jev call per message asks two questions at once: *chat, research or task?* and *does it need current facts from the web?*
  - Chat at 0.8 or more gets an instant answer from the fast model; research at 0.7 or more gets a cited web answer; anything else is a task. A chat that scores 0.6 or more on the web question gets a web search (OpenRouter's `web` plugin, about $0.007).
  - On the 45 labelled messages in `crates/waddle-core/tests/fixtures/router.json` it routed 44/45 correctly and got the web question right on 42/45, in one call (median 0.4 s, slowest 1.0 s, about $0.00002).
  - Tasks start before the answer comes back: the planner looks at the screen while Jev decides and is dropped silently if the message turns out to be chat or research. The first model call waits for the decision, so nothing on screen happens until it's a task.
  - Measured costs: a chat reply about $0.00002, a web chat about $0.007, a research answer about $0.008 (5 sources).

- **Mail importance (0.1.11):** Jev scores each new email from only the sender, subject and first line ("deserves interrupting the user now"). On the 30 labelled emails in `crates/waddle-core/tests/fixtures/importance.json`, all 18 that can wait scored 0.10 or less, and the 12 important ones scored 0.19–0.90. The nudge bar is therefore 0.4, not the planned 0.7: 29/30 right, no false alarms. The miss was "Call me when you can" from a parent (0.19). About $0.000015 per email.

**Jev Router (`typesafe/jev-router`)** is a different product: it picks a chat model for each request. It isn't tested here, because its per-request model choice makes cost unpredictable.

**Laya: later, for the local brain.**
- The same screen check, run locally, would save the local model about 55 s per text-only task. But it needs about 1.2 GB of RAM and an ARM64 onnxruntime build, and it has no GGUF version, so Ollama and LM Studio can't load it.
- Worth trying when local mode matters; the `Provider::needs_screen` hook is where it plugs in.

To rerun:

```bash
WADDLE_BENCH_CONCURRENCY=4 WADDLE_BENCH_REPEAT=2 WADDLE_BENCH_MODELS=openai/gpt-6-luna,google/gemini-3.8-flash … cargo test -p waddle-core --test bench -- --ignored --nocapture
```

## Candidates (prices)

Prices are per million input/output tokens, from OpenRouter's model list, October 2026:

| Model | Price | Why it's interesting |
|---|---|---|
| `qwen/qwen3.7-flash` | $0.03 / $0.13 | Newer Qwen vision model aimed at "computer interaction". A quarter of the current model's price. **First to try.** |
| `qwen/qwen3.8-flash` | $0.15 / $0.47 | Newest Qwen Flash, lists "desktop interaction". About the same price as today's default. |
| `qwen/qwen3.5-flash-02-23` | $0.065 / $0.26 | Qwen3.5 family (the local default's big sibling), half the price |
| `google/gemini-3.1-flash-lite` | $0.25 / $1.50 | Successor to the current quick-reply model |
| `openai/gpt-6-luna` | $0.10 / $0.50 | Cheap GPT-6 tier; uses pixel coordinates |
| `qwen/qwen3.8-27b` | $0.42 / $3.00 | Open weights; reported 84% on OSWorld-Verified. A "hard task" tier. |
| `google/gemini-3.8-flash` | $0.75 / $3.75 | Premium reference point |

Each model costs about $0.02–0.15 for a 21-task, 2-run pass.

**Watch the coordinate convention.** Waddle tells the model which grid to use:
- Qwen, Gemini and Gemma models get the 0–1000 grid.
- Everything else gets pixels.
- A model that picks its own convention will click in the wrong place until `coord_mode` is set in Settings.

**Reasoning models:** several of these think before answering. Settings now has a `reasoning` option (off / low / medium / high, sent as OpenRouter's `reasoning.effort`). Waddle's steps are small, so start with off or low; thinking adds seconds and cost to every step.

## Local (free) brain

| Model | Notes |
|---|---|
| `qwen3.5:4b` (current default) | **5/15 strict** in this container (4 x86 cores, CPU only), 190 s per task. In 6 of the failed tasks it clicked the right target, then kept re-checking (the test screen never changes) until the step limit. Its clicks are good. Its judgement is weak: it presses Enter after clicks, opens Notepad mid-task, and looks seven times instead of answering. Those are exactly what training on rated tasks fixes. A Snapdragon X should be roughly 2–3× faster. |
| Holo3.1-4B (H Company) | Qwen3.5-4B fine-tuned for computer use, sold on GUI grounding. GGUF with the vision projector bundled, on Ollama as a community upload (`ahmadwaqar/holo-3.1`). **The next local model to benchmark.** |
| Qwen3.8-Flash-Next | Newer Qwen with an Ollama library tag. Check its size: anything over about 5 GB hurts on a 16 GB Surface. |
| Fine-tuned Waddle model | See [TRAINING.md](TRAINING.md). A 4B model trained on your own rated tasks plus synthetic click data is the realistic way to make the free brain good. |

On Snapdragon X, local models run on the CPU (the Adreno GPU paths aren't usable yet). LM Studio runs Qwen3.5 faster than Ollama does.

## Sources

- OpenRouter model list and prices: `https://openrouter.ai/api/v1/models`, read October 4, 2026
- Qwen3.8 reported scores (OSWorld-Verified 84.3 for 27B): https://wavect.io/blog/qwen3-8-27b-self-hosted-computer-use-agents/, https://qwen.ai/blog?id=qwen3.8
- OSWorld leaderboards: https://benchlm.ai/benchmarks/osworld-verified, https://leaderboard.steel.dev/leaderboards/osworld-2/
- Holo3.1 GGUF: https://huggingface.co/g023/Holo-3.1-4B-GGUF, https://ollama.com/ahmadwaqar/holo-3.1
- Grounding RL (reward design): GUI-G1 https://github.com/Yuqi-Zhou/GUI-G1, FDC-Ground (AAAI), GuirlVG
