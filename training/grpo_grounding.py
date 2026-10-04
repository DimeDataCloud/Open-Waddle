"""Reinforcement fine-tuning (GRPO) for click accuracy, after SFT.

For each screenshot + instruction the model samples several answers; each
answer is scored by a rule (no reward model needed): did the click land in the
target box, and how close to its centre? GRPO pushes the model toward its own
better samples. This is the recipe of UI-R1 / GUI-R1 / GUI-G1, which lifted
3-7B models well past their SFT scores on ScreenSpot-style grounding with a few
thousand examples.

Data: grounding.jsonl from training/gen_grounding.mjs (each row has `box`, the
target on the 0-1000 grid). Start from the SFT LoRA for best results.

    python training/grpo_grounding.py --data grounding.jsonl --init out/lora --out out-grpo/
"""

import argparse
import json
import math
import re
from pathlib import Path

from PIL import Image

CALL = re.compile(r'"x"\s*:\s*(-?\d+(?:\.\d+)?)\s*,\s*"y"\s*:\s*(-?\d+(?:\.\d+)?)')


def click_reward(completion: str, box) -> float:
    """1.0 for a click at the box centre, ~0.6 at its edge, falling off outside;
    +0.1 for a well-formed click call. A smooth reward keeps GRPO's group
    advantages informative even when every sample lands inside the box."""
    m = CALL.search(completion)
    if not m:
        return 0.0
    x, y = float(m.group(1)), float(m.group(2))
    x0, y0, x1, y1 = box
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    # Distance in units of the box's half-size, so small icons and wide rows count alike.
    hw, hh = max(4.0, (x1 - x0) / 2), max(4.0, (y1 - y0) / 2)
    d2 = ((x - cx) / hw) ** 2 + ((y - cy) / hh) ** 2
    inside = x0 <= x <= x1 and y0 <= y <= y1
    return 0.1 + 0.6 * math.exp(-d2 / 2) + (0.3 if inside else 0.0)


def load(path, max_side):
    rows = []
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        r = json.loads(line)
        user = r["messages"][1]["content"]
        img = Image.open(next(p["image"] for p in user if p["type"] == "image")).convert("RGB")
        img.thumbnail((max_side, max_side))
        text = next(p["text"] for p in user if p["type"] == "text")
        rows.append(
            {
                "prompt": [
                    {"role": "system", "content": [{"type": "text", "text": r["messages"][0]["content"]}]},
                    {"role": "user", "content": [{"type": "image"}, {"type": "text", "text": text + '\nAnswer with a click call: {"x": ..., "y": ...}'}]},
                ],
                "image": img,
                "box": r["box"],
            }
        )
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--init", default="unsloth/Qwen3.5-4B", help="base model or the SFT LoRA folder")
    ap.add_argument("--out", default="out-grpo")
    ap.add_argument("--samples", type=int, default=6, help="answers per prompt (the GRPO group)")
    ap.add_argument("--steps", type=int, default=600)
    ap.add_argument("--max-side", type=int, default=1440)
    args = ap.parse_args()

    from trl import GRPOConfig, GRPOTrainer
    from unsloth import FastVisionModel

    model, tokenizer = FastVisionModel.from_pretrained(args.init, load_in_4bit=False, use_gradient_checkpointing="unsloth")
    model = FastVisionModel.get_peft_model(model, finetune_vision_layers=False, finetune_language_layers=True, r=16, lora_alpha=16)
    data = load(args.data, args.max_side)

    def reward(completions, box, **_):
        texts = [c if isinstance(c, str) else c[-1]["content"] for c in completions]
        return [click_reward(t, b) for t, b in zip(texts, box)]

    trainer = GRPOTrainer(
        model=model,
        processing_class=tokenizer,
        reward_funcs=[reward],
        train_dataset=data,
        args=GRPOConfig(
            output_dir=args.out,
            learning_rate=1e-6,
            per_device_train_batch_size=args.samples,
            num_generations=args.samples,
            max_completion_length=64,
            max_steps=args.steps,
            temperature=1.0,
            beta=0.0,
            logging_steps=10,
            bf16=True,
            report_to="none",
        ),
    )
    trainer.train()
    model.save_pretrained(Path(args.out) / "lora")
    tokenizer.save_pretrained(Path(args.out) / "lora")
    print(f"done: export with finetune_sft.py's GGUF step, or merge {args.out}/lora")


if __name__ == "__main__":
    main()
