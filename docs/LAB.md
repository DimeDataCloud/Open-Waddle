# The click lab (October 2026)

Which models should click Waddle's screens, and in which coordinate system? Leaderboards answer a different question (big models, other screens, other prompts), and a full benchmark run costs a few cents per model. The lab answers it for Waddle's own screens and prompt, at a fraction of a cent per model.

## What the research says (October 6, 2026)

- **No coordinate standard.** Each model family is trained on its own:
  - pixels of the screenshot: OpenAI's GPT models and Claude;
  - a 0–1000 grid: Gemini (boxes as `[ymin, xmin, ymax, xmax]`), Qwen3-VL (which moved away from Qwen2.5-VL's pixels) and Gemma;
  - 0–1 fractions: UGround and AGUVIS.
  Evaluations convert every answer to one space before scoring ([Phi-Ground](https://arxiv.org/html/2507.23779v1), [Qwen3-VL grounding](https://deepwiki.com/QwenLM/Qwen3-VL/5.2-spatial-understanding-and-2d-grounding)). Newer Qwen models don't document theirs, and Waddle's own benchmark caught `qwen/qwen3.7-flash` clicking at x = 1714 on a 1440-wide screen.
- **Grounding leaders are big and dear.** On [ScreenSpot-Pro](https://benchlm.ai/benchmarks/screenspot-pro) (high-resolution professional apps), GPT-6 Astra leads with 92.7% and Claude Opus 4.8 has 87.9%. The small specialists Holo2-4B and Holo2-8B score 57–59%. [MolmoPoint GUI](https://x.com/HuggingPapers/status/2036101402477404284) points with grounding tokens instead of coordinates (61.1%). None of the cheap generalists Waddle uses are listed.
- **Gemini 3.5 Flash-Lite** has computer use built in, and Google reports 74% on OSWorld-Verified ([Google](https://blog.google/innovation-and-ai/models-and-research/gemini-models/gemini-3-6-flash-3-5-flash-lite-3-5-flash-cyber/), [model card](https://deepmind.google/models/model-cards/gemini-3-5-flash-lite/)).
- **GPT-6 Luna**, Waddle's default planner, launched on September 22, 2026 with an image-encoding bug that OpenAI fixed on September 25 ([details](https://www.digitalapplied.com/blog/gpt-6-sol-luna-image-bug-fix-rerun-evals)). Waddle's Luna results date from October 4, after the fix. It costs $0.10 per million tokens in and $0.50 out ([pricing](https://tokencost.app/blog/gpt-6-luna-pricing)).
- **Accessibility first, pixels second.** Reading the accessibility tree is far cheaper than a screenshot but loses on its own (40% against 59% with vision on WebVoyager). Agents that try the accessibility action first and fall back to a pixel click do best ([overview](https://read.technically.dev/p/how-do-computer-use-agents-work)). Waddle already works this way (`find_elements`/`click_element`, then `click`).
- **OpenRouter's catalog** (pulled October 6) has about 270 models that take images and call tools. Several cost less than Luna, among them `qwen/qwen3.7-flash` ($0.03/$0.13), `inclusionai/ling-3.0-flash-vl` ($0.021/$0.062), `bytedance-seed/seed-1.6-flash` ($0.075/$0.30) and `google/gemma-4-26b-a4b-it` ($0.076/$0.255). A few are free. A free Google key also runs Gemma 4 (26B and 31B).

## How the lab works

`node bench/lab/probes.mjs` renders the six bench screens at 1440×960 and turns every piece of visible text that appears only once into a probe ("Click \"Network & internet\"."), plus the bench tasks whose first step is a click ("Open Grace's email about the Q3 budget"). That gives 86 probes, from 16×16 icons to whole rows.

`cargo test -p waddle-core --test lab -- --ignored --nocapture` (see the file's header) asks each model, in each coordinate system, one question per probe:
- the screenshot and the instruction;
- Waddle's own click tool, described for that coordinate system;
- Waddle's own parser, which reads the answer as the app would.

It is built to be cheap:
- Answers are kept in `bench/lab/results.jsonl` and never asked for again.
- A screening round of 10 probes drops what misses half of them.
- Only each model's better coordinate system goes on to the remaining probes.
- Free services go first, and paid calls stop at a budget.

The report (`bench/lab/REPORT.md`) fits a straight line from what each model said to where each target was. That shows which coordinate space a model really answers in, and how many clicks a fixed correction would save.

## Results (October 6, 2026)

Total cost of everything below: about **$0.06**:
- the lab's paid calls: $0.036;
- the follow-up checks on Ling: $0.01;
- a Gemini 2.5 Flash-Lite screen run from just before the lab: $0.014.

Google's free key and the free models cost nothing. The full table is in [bench/lab/REPORT.md](../bench/lab/REPORT.md).

### Clicks: first step, each model in its better coordinate system

| Model | Coordinates | Hits | Time | Per 1,000 clicks |
|---|---|---|---|---|
| `openai/gpt-6-luna` | pixels | **93%** (30) | 1.7 s | $0.23 |
| `gemini-3.5-flash-lite` (free key) | 0–1000 grid | **92%** (86) | 1.2 s | $0 |
| `gemma-4-31b-it` (free key) | 0–1000 grid | 91% (43) | 47 s | $0 |
| `xiaomi/mimo-v2.6-flash` | pixels | 90% (30) | 6.7 s | $0.24 |
| `inclusionai/ling-3.0-flash-vl` | 0–1000 grid | 83% (30) | 1.9 s | **$0.04** |
| `qwen/qwen3-vl-8b-instruct` | 0–1000 grid | 73% (30) | 0.9 s | $0.20 |
| `qwen/qwen3.7-flash` | either | 40–50% (10) | 5 s | $0.07–0.08 |
| `google/gemini-2.5-flash-lite` | either | 40% (10) | 1.9 s | $0.22 |

Out:
- `bytedance-seed/seed-1.6-flash` thinks until it runs out of tokens and never clicks.
- `gemma-4-26b-a4b-it` on Google returned server errors.
- OpenRouter's free models were rate-limited, restricted, or had no endpoint that honours Waddle's don't-train-on-my-data setting.

### What it showed

1. **Coordinates matter more than the model.**
   - Gemini 3.5 Flash-Lite hits 92% on a 0–1000 grid and 30% in pixels.
   - Luna hits 93% in pixels and 60% on the grid.
2. **Models answer in their own space whatever they're asked for.** Read as a 0–1000 grid, the answers models gave when asked for pixels hit:
   - Ling 90%,
   - Qwen3-VL 90%,
   - Qwen 3.7 Flash 80%.

   Read as pixels, Luna's answers when asked for the grid hit 80%. The fitted maps say the same: about x×1.44, y×0.96 (a 0–1000 grid) for Gemini, Gemma, Ling and Qwen, and ×1.00 (pixels) for Luna and MiMo. So Waddle must know each family's space. Auto now puts Ling with Gemini, Gemma and Qwen on the grid; before, it asked Ling for pixels (40%).
3. **Some answers were thrown away by Waddle, not missed by the model.** Qwen models sometimes:
   - garble the JSON (`{"x":": 380, "y": 352}`);
   - send a point pair where x should be (`"x": [564, 263]`, `{"x": 564, 263]`).

   Waddle now reads both, and `"coordinate": [x, y]`, the shape OpenAI's and Anthropic's computer-use tools use.
4. **Gemini's fallback must click on the grid too.** When Google turns a Gemini task down, the task's prompt and tools already say "0–1000". **Ling 3.0 Flash VL** is the cheapest model that clicks well there. Follow-up checks:
   - **11 of 12 assistant jobs** (4.3 s, $0.00027 a task). The miss was a careful one: it asked before deleting instead of calling the tool that asks.
   - **13 of 21 screen tasks**, 16 on target (6.4 s, $0.00031 a task), against 19 (21) for Gemini on the free key. Its misses land 10–16 px below row-shaped targets.

   It costs about a sixth of Gemini 3.5 Flash-Lite through OpenRouter and half of Luna. It's now the default fallback. Settings → Brain → **When Google turns a task down, OpenRouter runs** takes any other model, for example `google/gemini-3.5-flash-lite` for the best clicks at about six times the price.

Each number above comes from one run on six synthetic screens, so treat differences of a few points as noise. Add models and screens and run the lab again; answers already kept cost nothing.
