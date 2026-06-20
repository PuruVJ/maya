# Native-eval step 2: generate ops for each case using the fused model in its EXACT training format
# (Qwen2.5 chat template, NO grammar constraint) — the faithful signal, unlike node-llama-cpp's GBNF.
# Reads eval-cases.jsonl → writes eval-out.jsonl. Run with the mlx venv: .venv/bin/python
import json
from mlx_lm import load, generate

model, tok = load("training/fused")
with open("training/data/eval-out.jsonl", "w") as out:
    for line in open("training/data/eval-cases.jsonl"):
        c = json.loads(line)
        msgs = [{"role": "system", "content": c["system"]}, {"role": "user", "content": c["prompt"]}]
        prompt = tok.apply_chat_template(msgs, add_generation_prompt=True)
        raw = generate(model, tok, prompt=prompt, max_tokens=384, verbose=False).strip()
        try:
            ops = json.loads(raw).get("ops", [])
        except Exception:
            ops = []
        out.write(json.dumps({"i": c["i"], "ops": ops, "raw": raw[:400]}) + "\n")
print("✓ done → training/data/eval-out.jsonl")
