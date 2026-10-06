"""KTO: teach a model from your 👍 and 👎 alone, no paired answers needed.

Input: kto.jsonl from Settings -> Training data -> Export. Every reply of a
task you rated is one row, labelled with your thumb, and a reply that claimed
to do something no tool did is labelled bad even if you didn't rate it:

    {"prompt": [messages before the reply], "completion": [the reply], "label": true, "task_id": "..."}

KTO (Ethayarajh et al., 2024) learns from such unpaired desirable/undesirable
labels. Here it trains on the conversation text with the screenshots left out:
it shapes behaviour (call a tool instead of claiming, go straight to the
address, stop when done) rather than where to click, which SFT and GRPO cover.
Run it after finetune_sft.py, starting from its LoRA adapter.

    python training/finetune_kto.py --check kto.jsonl                  # no GPU: is the file usable?
    pip install -r training/requirements.txt                          # on a CUDA machine
    python training/finetune_kto.py --data kto.jsonl --init out/lora --out out-kto/
"""

import argparse
import json
import sys
from pathlib import Path


def text_of(content):
    """Message content (a string, or parts with text and images) -> its text; images are dropped."""
    if isinstance(content, str):
        return content
    return "\n".join(p.get("text", "") for p in content or [] if p.get("type") != "image").strip()


def to_text_message(m):
    out = {"role": m["role"], "content": text_of(m.get("content"))}
    if m.get("tool_calls"):
        out["tool_calls"] = [
            {"type": "function", "function": {"name": c["function"]["name"], "arguments": json.loads(c["function"].get("arguments") or "{}")}}
            for c in m["tool_calls"]
        ]
    if m.get("tool_call_id"):
        out["tool_call_id"] = m["tool_call_id"]
    return out


def load_rows(paths):
    rows = []
    for path in paths:
        for n, line in enumerate(Path(path).read_text(encoding="utf-8").splitlines(), 1):
            if not line.strip():
                continue
            r = json.loads(line)
            rows.append(
                {
                    "prompt": [to_text_message(m) for m in r["prompt"]],
                    "completion": [to_text_message(m) for m in r["completion"]],
                    "label": bool(r["label"]),
                    "task_id": r.get("task_id", f"{path}:{n}"),
                }
            )
    return rows


def problems(rows):
    """What would make training fail or learn nothing."""
    found = []
    for i, r in enumerate(rows):
        if not r["prompt"] or r["prompt"][-1]["role"] not in ("user", "tool"):
            found.append(f"row {i}: the prompt must end with the user's message or a tool result")
        if len(r["completion"]) != 1 or r["completion"][0]["role"] != "assistant":
            found.append(f"row {i}: the completion must be one assistant reply")
        elif not r["completion"][0]["content"] and not r["completion"][0].get("tool_calls"):
            found.append(f"row {i}: an empty reply")
    good = sum(r["label"] for r in rows)
    bad = len(rows) - good
    if not rows:
        found.append("no rows: rate some tasks 👍/👎 first")
    elif min(good, bad) == 0:
        found.append(f"only one kind of label ({good} good, {bad} bad): KTO needs both")
    return found


def summary(rows):
    good = sum(r["label"] for r in rows)
    tasks = len({r["task_id"] for r in rows})
    calls = sum(bool(r["completion"][0].get("tool_calls")) for r in rows if r["completion"])
    return f"{len(rows)} rows from {tasks} tasks: {good} good, {len(rows) - good} bad; {calls} replies call tools"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", nargs="+", help="only validate these files and print a summary (no GPU needed)")
    ap.add_argument("--data", nargs="+")
    ap.add_argument("--init", help="LoRA adapter from finetune_sft.py to start from")
    ap.add_argument("--base", default="unsloth/Qwen3.5-4B")
    ap.add_argument("--out", default="out-kto")
    ap.add_argument("--epochs", type=float, default=1)
    ap.add_argument("--lr", type=float, default=5e-6)
    ap.add_argument("--beta", type=float, default=0.1)
    args = ap.parse_args()

    paths = args.check or args.data
    if not paths:
        ap.error("give --check FILE or --data FILE")
    rows = load_rows(paths)
    print(summary(rows))
    issues = problems(rows)
    for p in issues[:20]:
        print("  -", p)
    if args.check:
        sys.exit(1 if issues else 0)
    if issues:
        sys.exit("fix the data first")

    # Imported here so --check and --help work without a GPU stack installed.
    from datasets import Dataset
    from trl import KTOConfig, KTOTrainer
    from unsloth import FastLanguageModel

    model, tokenizer = FastLanguageModel.from_pretrained(args.init or args.base, load_in_4bit=False)
    if not args.init:
        model = FastLanguageModel.get_peft_model(model, r=16, lora_alpha=16, lora_dropout=0.0)
    good = sum(r["label"] for r in rows)
    bad = len(rows) - good
    data = Dataset.from_list([{k: r[k] for k in ("prompt", "completion", "label")} for r in rows])
    trainer = KTOTrainer(
        model=model,
        processing_class=tokenizer,
        train_dataset=data,
        args=KTOConfig(
            output_dir=f"{args.out}/checkpoints",
            beta=args.beta,
            # Weigh the rarer label up so both count about the same (the KTO paper's advice).
            desirable_weight=max(1.0, bad / max(good, 1)),
            undesirable_weight=max(1.0, good / max(bad, 1)),
            per_device_train_batch_size=1,
            gradient_accumulation_steps=8,
            num_train_epochs=args.epochs,
            learning_rate=args.lr,
            max_length=8192,
            bf16=True,
            logging_steps=10,
            report_to="none",
        ),
    )
    trainer.train()
    out = Path(args.out)
    model.save_pretrained(out / "lora")
    tokenizer.save_pretrained(out / "lora")
    print(f"done: {out / 'lora'}; make a GGUF with finetune_sft.py's export step, then compare with bench/")


if __name__ == "__main__":
    main()
