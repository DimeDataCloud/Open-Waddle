# Making the model better at Waddle's job

There are two levers, in order of payoff:

1. **The harness** (prompt, tools, what the model sees, guard-rails in code). It's cheap, changes take effect immediately, and every model benefits.
   - It took the default model from 27% to 93% on our benchmark ([MODELS.md](MODELS.md)).
   - Always measure a harness change with `bench/` before and after.
2. **Training** a small model on Waddle's exact work: clicks on real screens and short tool-calling steps.
   - This is how the free local brain gets good.
   - A hosted model can't be fine-tuned through OpenRouter, so a fine-tuned model runs locally (Ollama / LM Studio) or on a GPU host you pay for.

## 1. Harness: what's in place

| Piece | Where | Why |
|---|---|---|
| Look first: window list, accessibility elements and a screenshot attached to every task | `Agent::observe` | Small models refuse or guess blind without it |
| Name-checked `click_element` | `Agent::check_element` | Models name the right button but send a neighbouring id |
| No third identical click or keypress in a row | `Agent::handle_call` | Breaks dead-button loops |
| Nudge when a reply announces an action but calls no tool | `promises_action` | "I'll click it" with nothing done |
| No blind input before any look | `Agent::handle_call` | Keystrokes landing in the wrong app |
| One screenshot kept for hosted models, a few for local ones | `prune_images`, `image_budget` | Cost, and the local prompt cache |
| Coordinate grid per model family | `CoordMode` | Qwen and Gemini answer on 0–1000, others in pixels |
| Reasoning effort setting | `Settings::reasoning` | Thinking models are slow per step |
| One nudge when a reply claims something no tool did, and the chat lane's claims sent to a task | `claims_action`, `HandOffGate` | "I've opened YouTube" with nothing opened (9 of 14 such tasks in the first real traces) |
| Several calls in one reply; the rest skipped after a failure | `Agent::run` | Each model step was most of the wait (3.2 s median in real use) |
| Read the page in the same step (`read` on browser tools), search addresses in the tool description | `tools::browser` | A read after every click and typing into search boxes doubled the steps |
| `arrange_window`, and keys that only move around without the countdown | `tools::arrange`, `classify_keys` | A split screen took 13 key presses at 2 s each |
| App names matched by sound and spelling | `heard::match_app` | Speech wrote "Clawed" for Claude |
| Workflow memory: steps that worked, offered for similar requests | `experience.rs` | Learning without a GPU (below) |

### Learning without a GPU: workflow memory

Waddle gets better at your tasks as you use it, with no training run (the idea of [Agent Workflow Memory](https://arxiv.org/abs/2409.07429), which raised web-agent success 24–51% relative with fewer steps):

1. When a task finishes without asking anything back, its tool calls are condensed into a short recipe ("browser_tabs open music.youtube.com in a new window, reading its elements → browser_click the element you need → arrange_window right_half on the right monitor"). Failed and skipped calls are left out.
2. Only what Waddle itself chose is kept: tool names, app names, keys, and web addresses without their query (only for well-known sites or a site your request named, so an address a page suggested isn't remembered). Typed text, email bodies and anything a page or file said never are, so nothing untrusted can come back as advice.
3. When a new request shares enough content words with an earlier one, the closest recipe rides along with the request as "a way that worked before (adapt it)".
4. 👍 confirms a recipe (it ranks a little higher), 👎 deletes it, and **Forget conversation** clears them all. They're kept in `experience.json` in the app data folder (200 at most).

### Next harness experiments (measure each with `bench/`)

1. **Set-of-Mark screenshots on Windows.** Draw the UI Automation element boxes and ids onto the screenshot, so the model can pick by number what it sees. It's the standard boost for small models on dense screens.
2. **Zoom.** For a small target, crop around the model's first guess at 2× and ask again. This fixes Gemini-style 20–40 px misses.
3. **Check after acting.** After a click, compare the next screenshot with the last. If nothing changed, say so ("nothing happened; the click may have missed").
4. **A cheap router.** Send text-only steps (file and command work) to a tiny text model and screen steps to the vision model.

## 2. Training: the pipeline

```
real tasks you rate 👍 ─┐
synthetic clicks ───────┼─> train.jsonl ─> SFT (LoRA) ─> GRPO on clicks ─> KTO on 👍/👎 ─> GGUF ─> Ollama "waddle-duck"
teacher-model successes ┘                                                ▲                     └─> bench/ to compare
real tasks you rate 👍 or 👎, and false claims ─────> kto.jsonl ──────────┘
```

### Data source A: your own tasks (best quality)

1. Settings → **Training data** → tick **Save my tasks for training**.
   - Each finished task is saved under the app data folder (`traces/<task>/trace.json` plus `screen-N.png`): the full conversation, the tools offered, the model, the outcome, tokens and cost.
2. After a task, the bubble asks **"Did that work? 👍 👎"**.
3. **Export training file** writes two files to `Documents\Waddle\training\`:
   - `train.jsonl` (for SFT): 👍 tasks, and finished tasks you didn't rate. Never 👎 tasks, tasks that failed or were halted, or tasks whose reply claimed something no tool did. The same conversation saved twice counts once.
   - `kto.jsonl` (for KTO): every reply of a task you rated, labelled with your 👍/👎, plus replies that claimed something no tool did, labelled bad.
   - **Personal details are masked** in the text: email addresses and the names in email headers, phone numbers, your user folder name, your saved facts and your email style note. Names inside web pages, emails and file names can't all be found, so look through the files before sharing them.
   - To export a folder of saved tasks without the app: `WADDLE_TRACES=<folder> WADDLE_EXPORT=<out> cargo test -p waddle-core --test traces_export -- --ignored --nocapture`.
4. Screenshots never leave the computer unless you copy them somewhere. The files point at them by path; they aren't masked.
5. Recording is off by default, and should stay off on screens with private information.

### Data source B: synthetic clicks (unlimited, free)

```bash
PLAYWRIGHT=/path/to/playwright/index.mjs node training/gen_grounding.mjs urls.txt out/grounding
```

What it does:
1. Opens each URL in Google Chrome, at the screen sizes Waddle sees, in light or dark theme.
2. Finds every visible, uniquely named button, link, field and tab.
3. Writes one example per element: the screenshot, an instruction ("Click Compose"), and the answer as Waddle's own `click` call on the 0–1000 grid, plus the target box for the RL reward.

Notes:
- This is the SeeClick / OS-Atlas / UGround recipe. It was checked here on Wikipedia, Hacker News, GitHub and MDN, and the boxes line up with the elements.
- A few hundred varied URLs give tens of thousands of examples.
- **Never** include `bench/pages`: training on the benchmark makes its scores meaningless.

### Data source C: a teacher model's successes (distillation)

```bash
WADDLE_BENCH_TRACES=out/teacher WADDLE_BENCH_MODELS=google/gemini-3.8-flash … cargo test -p waddle-core --test bench -- --ignored
```

How it works:
- Every *passing* run is saved as a rated trace. Failures are dropped: this is rejection sampling.
- Point it at a task set made for training (copy `bench/` and change the pages and tasks), not at the benchmark itself.
- A big model's correct behaviour becomes the small model's lessons.

### Step 1: supervised fine-tuning (LoRA)

```bash
pip install -r training/requirements.txt          # on a CUDA machine
python training/finetune_sft.py --data train.jsonl out/grounding/grounding.jsonl --out out/ --base unsloth/Qwen3.5-4B
ollama create waddle-duck -f out/Modelfile
```

- **Base model:** Qwen3.5-4B, the local default. Holo3.1-4B (already tuned for computer use) is also a good start.
- **Hardware:** about 10 GB of VRAM in bf16 LoRA. A rented RTX 4090 or L4 is about $0.40–0.80 an hour, and an epoch over ~5,000 examples takes under an hour.
- **What gets trained:** both the vision and language layers, since grounding lives partly in the vision tower. Loss is computed on the model's replies only.
- **Output:** a LoRA adapter, then a Q4_K_M GGUF with its vision projector, plus an Ollama `Modelfile`.

### Step 2: reinforcement on clicks (GRPO)

```bash
python training/grpo_grounding.py --data out/grounding/grounding.jsonl --init out/lora --out out-grpo/
```

How it works:
- The model samples 6 answers per screenshot, and each is scored by a rule: click inside the box gets more, at the centre gets the most (`click_reward`; `python training/test_rewards.py` checks it).
- No reward model and no human labels are needed.
- This is the UI-R1 / GUI-R1 / GUI-G1 recipe, which lifted 3–7B models well past their SFT scores on ScreenSpot-style grounding.

### Step 2b: learn from thumbs (KTO)

```bash
python training/finetune_kto.py --check kto.jsonl                    # no GPU needed: is the file usable?
python training/finetune_kto.py --data kto.jsonl --init out/lora --out out-kto/
```

- **KTO** (Kahneman-Tversky Optimisation) learns from unpaired good/bad labels, which is exactly what 👍/👎 are; no second "better answer" is needed per task.
- It trains on the conversation text (screenshots left out), so it shapes behaviour: call a tool instead of claiming, go straight to the address, stop when done. Clicking is SFT's and GRPO's job.
- The rarer label is weighted up so both count about the same. `--check` refuses a file with only one kind of label.
- On the first three batches of real traces, the export gave 106 labelled replies from 18 tasks (77 good, 29 bad): enough to check the pipeline, far too few to train on. Keep rating.

### Step 3: compare, then switch

```bash
WADDLE_BENCH_PROVIDER=ollama WADDLE_BENCH_MODELS=qwen3.5:4b,waddle-duck cargo test -p waddle-core --test bench -- --ignored --nocapture
```

If `waddle-duck` beats the base model:
1. In Settings, pick the **Ollama** preset and model `waddle-duck`.
2. Keep recording and rating.
3. Retrain every few hundred new tasks.

## Why not train the cloud model?

- **The model:** Qwen3-VL-8B on OpenRouter is a shared, hosted model. Its weights can't be swapped for ours there.
- **The paid route:** fine-tune the same open model and host it on a GPU inference provider. That costs money per hour or per token, at a markup over OpenRouter.
- **The free route:** the local 4B model costs nothing to run. It's the one where training changes the most: small models gain the most from task-specific data.

## Sources

- Unsloth Qwen3.5 fine-tuning (vision, RL, GGUF export, VRAM table): https://unsloth.ai/docs/models/qwen3.5/fine-tune
- GUI-G1 (R1-Zero-style RL for GUI grounding): https://github.com/Yuqi-Zhou/GUI-G1
- Grounding reward design (Gaussian click reward, GRPO for grounding): FDC-Ground, AAAI 2026, https://ojs.aaai.org/index.php/AAAI/article/view/40038; GuirlVG, https://www.emergentmind.com/topics/guirlvg
- Small-model grounding by distillation: WinDOM, https://arxiv.org/pdf/2606.25964
