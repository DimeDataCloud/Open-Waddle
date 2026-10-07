# Click lab report

Written by `crates/waddle-core/tests/lab.rs` from every answer in `results.jsonl` (see that file's header for how to run it). One probe is one screenshot of a bench screen (1440×960) and one instruction; a hit is a click inside the target.

- **Best reading:** the model's answers read as pixels and as a 0–1000 grid, whatever it was asked for; the better one is the space it really answers in. Above **Hits** means Waddle should ask it for that space.
- **Fitted map:** a line through what it said and where the targets were, fitted by medians (pixels: ×1.00+0; a 0–1000 grid: x×1.44, y×0.96).
- **Per 1,000 clicks:** what OpenRouter charged; Google's free key and `:free` models are $0.

| Model | Coordinates | Probes | Hits | Small targets | Task goals | Median miss | Best reading | Fitted map | Time | Per 1,000 clicks | No click |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `openai/gpt-6-luna` | pixels | 30 | **93%** | 100% of 4 | 100% of 2 | 0 px | pixels 93% | x×0.96+59, y×1.00+0 | 1.7 s | $0.234 | 0 |
| `gemini-3.5-flash-lite` | norm1000 | 86 | **92%** | 85% of 13 | 100% of 7 | 0 px | 0–1000 grid 92% | x×1.42+50, y×0.96+0 | 1.2 s | $0.000 | 0 |
| `gemma-4-31b-it` | norm1000 | 43 | **91%** | 88% of 8 | 100% of 3 | 0 px | 0–1000 grid 91% | x×1.43+7, y×0.96+3 | 46.8 s | $0.000 | 4 |
| `xiaomi/mimo-v2.6-flash` | pixels | 30 | **90%** | 100% of 4 | 50% of 2 | 0 px | pixels 90% | x×0.95+64, y×1.00+2 | 6.7 s | $0.237 | 1 |
| `inclusionai/ling-3.0-flash-vl` | norm1000 | 30 | **83%** | 75% of 4 | 100% of 2 | 0 px | 0–1000 grid 83% | x×1.44+3, y×0.96+0 | 1.9 s | $0.042 | 2 |
| `qwen/qwen3-vl-8b-instruct` | norm1000 | 30 | **73%** | 75% of 4 | 100% of 2 | 0 px | 0–1000 grid 73% | x×1.39+44, y×0.96+0 | 0.9 s | $0.203 | 2 |
| `openai/gpt-6-luna` | norm1000 | 10 | **60%** | 0% of 2 | 100% of 2 | 0 px | pixels 80% | x×1.18+79, y×1.00+0 | 1.7 s | $0.235 | 0 |
| `qwen/qwen3.7-flash` | norm1000 | 10 | **50%** | 100% of 2 | 0% of 2 | 1 px | 0–1000 grid 50% | x×1.27+90, y×0.96+0 | 5.3 s | $0.079 | 0 |
| `xiaomi/mimo-v2.6-flash` | norm1000 | 10 | **50%** | 0% of 2 | 50% of 2 | 0 px | pixels 50% | x×0.97+129, y×1.00-2 | 6.1 s | $0.263 | 2 |
| `inclusionai/ling-3.0-flash-vl` | pixels | 10 | **40%** | 0% of 2 | 100% of 2 | 67 px | 0–1000 grid 90% | x×1.43+5, y×0.96+1 | 2.0 s | $0.043 | 0 |
| `qwen/qwen3.7-flash` | pixels | 10 | **40%** | 50% of 2 | 50% of 2 | 20 px | 0–1000 grid 80% | x×1.28+72, y×0.96+3 | 4.5 s | $0.071 | 0 |
| `qwen/qwen3-vl-8b-instruct` | pixels | 10 | **40%** | 0% of 2 | 100% of 2 | 45 px | 0–1000 grid 90% | x×1.40+21, y×0.96-1 | 1.0 s | $0.203 | 0 |
| `google/gemini-2.5-flash-lite` | norm1000 | 10 | **40%** | 0% of 2 | 100% of 2 | 3 px | 0–1000 grid 40% | x×1.28+164, y×0.97+0 | 1.9 s | $0.216 | 1 |
| `google/gemini-2.5-flash-lite` | pixels | 10 | **40%** | 0% of 2 | 100% of 2 | 62 px | 0–1000 grid 70% | x×1.38+67, y×0.97-1 | 1.9 s | $0.234 | 0 |
| `gemini-3.5-flash-lite` | pixels | 10 | **30%** | 0% of 2 | 50% of 2 | 89 px | 0–1000 grid 90% | x×1.35+86, y×0.96-1 | 1.2 s | $0.000 | 0 |
| `gemma-4-31b-it` | pixels | 10 | **20%** | 0% of 2 | 50% of 2 | 27 px | 0–1000 grid 70% | – | 37.8 s | $0.000 | 3 |
| `dots-studio/dots-3-note-preview:free` | pixels | 3 | **0%** | 0% of 1 | 0% of 1 | NaN px | – | – | 12.8 s | $0.000 | 3 |
| `gemma-4-26b-a4b-it` | pixels | 7 | **0%** | 0% of 2 | 0% of 2 | 273 px | 0–1000 grid 43% | – | 7.4 s | $0.000 | 4 |
| `google/gemma-4-31b-it:free` | pixels | 3 | **0%** | 0% of 1 | 0% of 1 | NaN px | – | – | 4.8 s | $0.000 | 3 |
| `nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free` | pixels | 3 | **0%** | 0% of 1 | 0% of 1 | NaN px | – | – | 0.3 s | $0.000 | 3 |
| `thinkingmachines/inkling-small:free` | pixels | 3 | **0%** | 0% of 1 | 0% of 1 | NaN px | – | – | 0.2 s | $0.000 | 3 |
| `bytedance-seed/seed-1.6-flash` | pixels | 3 | **0%** | 0% of 1 | 0% of 1 | NaN px | – | – | 6.4 s | $0.404 | 3 |
