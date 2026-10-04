# Choosing Waddle's brain (October 2026)

Short version:
- **Keep `qwen/qwen3-vl-8b-instruct` as the cloud default.** With the new harness it passes 93% of our desktop tasks at about $0.001 per task.
- **The harness mattered far more than the model.** The same model scored 27% before.
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

## Candidates we couldn't test (yet)

The OpenRouter key's guardrail only allows the two models above. Every other model returns "blocked by guardrail". These look most promising on paper; prices are per million input/output tokens from OpenRouter's model list, October 2026:

| Model | Price | Why it's interesting |
|---|---|---|
| `qwen/qwen3.7-flash` | $0.03 / $0.13 | Newer Qwen vision model aimed at "computer interaction". A quarter of the current model's price. **First to try.** |
| `qwen/qwen3.8-flash` | $0.15 / $0.47 | Newest Qwen Flash, lists "desktop interaction". About the same price as today's default. |
| `qwen/qwen3.5-flash-02-23` | $0.065 / $0.26 | Qwen3.5 family (the local default's big sibling), half the price |
| `google/gemini-3.1-flash-lite` | $0.25 / $1.50 | Successor to the current quick-reply model |
| `openai/gpt-6-luna` | $0.10 / $0.50 | Cheap GPT-6 tier; uses pixel coordinates |
| `qwen/qwen3.8-27b` | $0.42 / $3.00 | Open weights; reported 84% on OSWorld-Verified. A "hard task" tier. |
| `google/gemini-3.8-flash` | $0.75 / $3.75 | Premium reference point |

To test them:
1. Allow them at openrouter.ai → Workspaces → Guardrails.
2. Run the bench command above with `WADDLE_BENCH_MODELS` set to the list.

Each model costs about $0.01–0.05 for a 15-task pass.

**Watch the coordinate convention.** Waddle tells the model which grid to use:
- Qwen, Gemini and Gemma models get the 0–1000 grid.
- Everything else gets pixels.
- A model that picks its own convention will click in the wrong place until `coord_mode` is set in Settings.

**Reasoning models:** several of these think before answering. Settings now has a `reasoning` option (off / low / medium / high, sent as OpenRouter's `reasoning.effort`). Waddle's steps are small, so start with off or low; thinking adds seconds and cost to every step.

## Local (free) brain

| Model | Notes |
|---|---|
| `qwen3.5:4b` (current default) | Benchmark result in this container (4 x86 cores, CPU only): see `bench/results/qwen3.5_4b@local-cpu.json`. Each screenshot step takes about 60 s here; a Snapdragon X runs roughly 2–3× faster. |
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
