"""Supervised fine-tuning (LoRA) of a small vision model on Waddle data.

Inputs are JSONL files in chat format with image paths:
  - train.jsonl from Settings -> Training data -> Export (real tasks you rated 👍)
  - grounding.jsonl from training/gen_grounding.mjs (synthetic clicks)

Output: a LoRA adapter, a merged model, and a Q4_K_M GGUF plus an Ollama
Modelfile, so `ollama create waddle-duck -f out/Modelfile` gives Waddle a local
brain trained on your own work.

Needs one CUDA GPU: about 10 GB of VRAM for Qwen3.5-4B in bf16 LoRA (a rented
RTX 4090 or L4 for an hour is enough for a few thousand examples).

    pip install -r training/requirements.txt
    python training/finetune_sft.py --data train.jsonl grounding.jsonl --out out/
"""

import argparse
import json
import random
from pathlib import Path

from PIL import Image


def load_examples(paths, max_side):
    examples = []
    for path in paths:
        for line in Path(path).read_text().splitlines():
            if not line.strip():
                continue
            row = json.loads(line)
            examples.append({"messages": [to_unsloth(m, max_side) for m in row["messages"]], "tools": row.get("tools") or []})
    random.shuffle(examples)
    return examples


def to_unsloth(message, max_side):
    """OpenAI-style message -> the content-list form Unsloth's vision collator reads, images loaded."""
    content = message.get("content") or ""
    if isinstance(content, str):
        parts = [{"type": "text", "text": content}] if content else []
    else:
        parts = []
        for p in content:
            if p["type"] == "image":
                img = Image.open(p["image"]).convert("RGB")
                # Same downscale Waddle's screenshots get at 200% scaling; keeps tokens per image sane.
                img.thumbnail((max_side, max_side))
                parts.append({"type": "image", "image": img})
            else:
                parts.append({"type": "text", "text": p["text"]})
    out = {"role": message["role"], "content": parts}
    if message.get("tool_calls"):
        out["tool_calls"] = [
            {"type": "function", "function": {"name": c["function"]["name"], "arguments": json.loads(c["function"]["arguments"] or "{}")}}
            for c in message["tool_calls"]
        ]
    if message.get("tool_call_id"):
        out["tool_call_id"] = message["tool_call_id"]
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", nargs="+", required=True)
    ap.add_argument("--out", default="out")
    ap.add_argument("--base", default="unsloth/Qwen3.5-4B", help="any Unsloth vision model, e.g. unsloth/Qwen3-VL-8B-Instruct")
    ap.add_argument("--epochs", type=float, default=2)
    ap.add_argument("--lr", type=float, default=1e-4)
    ap.add_argument("--rank", type=int, default=16)
    ap.add_argument("--max-side", type=int, default=1440)
    ap.add_argument("--quant", default="q4_k_m")
    args = ap.parse_args()

    # Imported here so --help works without a GPU stack installed.
    from trl import SFTConfig, SFTTrainer
    from unsloth import FastVisionModel
    from unsloth.trainer import UnslothVisionDataCollator

    model, tokenizer = FastVisionModel.from_pretrained(args.base, load_in_4bit=False, use_gradient_checkpointing="unsloth")
    model = FastVisionModel.get_peft_model(
        model,
        finetune_vision_layers=True,  # grounding lives partly in the vision tower
        finetune_language_layers=True,
        r=args.rank,
        lora_alpha=args.rank,
        lora_dropout=0.0,
    )
    data = load_examples(args.data, args.max_side)
    print(f"{len(data)} examples")
    FastVisionModel.for_training(model)
    trainer = SFTTrainer(
        model=model,
        tokenizer=tokenizer,
        data_collator=UnslothVisionDataCollator(model, tokenizer, train_on_responses_only=True),
        train_dataset=data,
        args=SFTConfig(
            output_dir=f"{args.out}/checkpoints",
            per_device_train_batch_size=1,
            gradient_accumulation_steps=8,
            num_train_epochs=args.epochs,
            learning_rate=args.lr,
            warmup_ratio=0.05,
            lr_scheduler_type="cosine",
            logging_steps=10,
            bf16=True,
            remove_unused_columns=False,
            dataset_text_field="",
            dataset_kwargs={"skip_prepare_dataset": True},
            max_length=8192,
            report_to="none",
        ),
    )
    trainer.train()

    out = Path(args.out)
    model.save_pretrained(out / "lora")
    tokenizer.save_pretrained(out / "lora")
    # GGUF (with the vision projector) for Ollama / LM Studio on the Surface.
    model.save_pretrained_gguf(str(out / "gguf"), tokenizer, quantization_method=args.quant)
    ggufs = sorted((out / "gguf").glob("*.gguf"))
    text = [g for g in ggufs if "mmproj" not in g.name]
    proj = [g for g in ggufs if "mmproj" in g.name]
    lines = [f"FROM ./gguf/{text[0].name}" if text else "FROM ./gguf"]
    if proj:
        lines.append(f"ADAPTER ./gguf/{proj[0].name}")
    lines += ["PARAMETER temperature 0.2", "PARAMETER num_ctx 8192"]
    (out / "Modelfile").write_text("\n".join(lines) + "\n")
    print(f"done: ollama create waddle-duck -f {out / 'Modelfile'}  then pick Ollama + waddle-duck in Waddle's Settings")


if __name__ == "__main__":
    main()
