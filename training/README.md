# Specialising the world-builder model (local, on an M4 Max)

Goal: replace the general chat model with a small model trained **only** on our task — natural
language → our exact ops grammar, and a clean refusal for everything else. Wins: **smaller → less
GPU jank**, **higher accuracy** on counts/kinds/anchors, **disciplined refusals** instead of
improvising. Runtime stays 100% local (WebLLM/WebGPU); the only "cloud" is an optional Hugging Face
repo to *host* the weights for prod.

We ship **two** fine-tunes, both pickable in-app: **WorldGen-1.5B** (Qwen2.5-1.5B, sharpest) and
**WorldGen-0.5B** (Qwen2.5-0.5B, ~280 MB — for low-end / mobile GPUs). Same pipeline, different `BASE`.

```
Phase 0  refusal/off-topic discipline ........ already in src/lib/llm-prompt.ts (few-shots)
Phase 1  synthetic dataset .................... pnpm gen:dataset  → training/data/*.jsonl
Phase 2  train → fuse → convert → host → wire . training/*.sh
```

Everything reuses the **real** grammar + validator + engine, so the dataset can never drift from
production.

---

## Prerequisites

- macOS on Apple Silicon (M4 Max is plenty — a 1.5B LoRA is light; 0.5B lighter still).
- Python 3.11+ in a venv, and Node/pnpm (already set up for the app).
- **Docker** with `linux/amd64` emulation (OrbStack or Docker Desktop) — only for the MLC conversion.

```bash
python3 -m venv .venv && source .venv/bin/activate
pip install -U mlx-lm                         # training + fuse (Apple MLX)
# optional, only to score a GGUF in our battery:
#   git clone https://github.com/ggml-org/llama.cpp && (cd llama.cpp && cmake -B build && cmake --build build -j)
```

> **No local MLC install.** Conversion to the browser format runs entirely inside a pinned Docker
> container (`convert-mlc.sh`). We tried building MLC/TVM from source on arm64 — its LLVM **codegen
> never registers** (dead end). The container instead uses MLC's *prebuilt x86_64* `tvm` wheel (codegen
> works) + `mlc_llm`'s python from source, with `mlc-prebuilt.patch` making the few C++-only import
> paths that `convert_weight`/`gen_config` don't need optional. Pinned: `mlc-ai-nightly-cpu==0.26.dev61`,
> `mlc-llm@2008fe8`. See `_mlc_convert.sh`.

---

## Phase 1 — generate the dataset

```bash
pnpm gen:dataset            # → training/data/train.jsonl (5000) + valid.jsonl (400)
pnpm gen:dataset 8000 600   # bigger, if you want
```

Each line is a chat example `{"messages":[system, user, assistant]}` where the **system is the compact
`buildWorldState()`** the tuned models use at runtime (live world state only — objects/ids/player/
ground/sky, no vocabulary or few-shots, since the model internalises the grammar), the **user** is a
messy human prompt (typos, casing, slang, vague vibes, refusals, off-topic junk), and the **assistant**
is the correct `{"ops":[…]}`. Every target is validated through `isValidOp` + `applyOps` before it's
written, and CRUD examples are id-grounded against a real seeded world. Tune category weights / banks
at the top of `training/gen-dataset.ts`.

---

## Phase 2 — train, convert, host, wire

```bash
cd training
chmod +x *.sh        # first time

# --- WorldGen-1.5B (default) -------------------------------------------------
./train_lora.sh                                   # 2a  LoRA fine-tune. ~20–40 min on M4 Max → ./adapters
./fuse.sh                                         # 2b  merge adapters → standalone HF model → ./fused
./convert-mlc.sh                                  # 2c  → training/dist/WorldGen-1.5B + static/models/…

# --- WorldGen-0.5B (lighter alternative) -------------------------------------
BASE=Qwen/Qwen2.5-0.5B-Instruct ADAPTERS=./adapters-0.5b ./train_lora.sh
BASE=Qwen/Qwen2.5-0.5B-Instruct ADAPTERS=./adapters-0.5b FUSED=./fused-0.5b ./fuse.sh
FUSED=./fused-0.5b NAME=WorldGen-0.5B ./convert-mlc.sh
```

Each variant keeps separate `adapters*/` `fused*/` and `dist/<NAME>` dirs so they never collide. Other
training knobs: `ITERS=4000 ./train_lora.sh`. Conversion is **reproducible** — `convert-mlc.sh` always
spins a fresh pinned container, so re-running yields identical weights.

### Evaluate it (recommended before shipping)

Convert to GGUF and run the **existing battery** — it scores the tuned model next to the stock 1.5B/3B
on intent, compound/CRUD, and the messy scenario suite:

```bash
./to_gguf.sh         # needs llama.cpp at $LLAMA_CPP (default ~/llama.cpp) → .models/worldgen-*.gguf
cd .. && pnpm test:llm
```

> Note: GBNF-grammar battery scoring under-counts the fine-tune; the **native** MLX generate
> (`eval-native.py`) is the faithful signal (the 1.5B scored 44/49, matching stock 3B).

### Host + turn it on

For **local dev**, `convert-mlc.sh` already copies the weights to `static/models/<NAME>/`, so they're
served origin-relative — nothing else to do. For **prod**, upload the dist folder to a Hugging Face repo
(WebLLM loads MLC models from HF URLs natively — free CDN + CORS):

```bash
huggingface-cli login                                              # once, WRITE token
huggingface-cli upload <user>/WorldGen-1.5B training/dist/WorldGen-1.5B .
```

Then point the matching entry's `url` in the `TUNED` map in `src/lib/llm.svelte.ts` at the HF
`/resolve/main/` URL (trailing slash):

```ts
const TUNED: Partial<Record<ModelKey, TunedDef>> = {
  tuned:      { id: 'WorldGen-1.5B', url: 'https://huggingface.co/<user>/WorldGen-1.5B/resolve/main/', stockId: 'Qwen2.5-1.5B-Instruct-q4f16_1-MLC' },
  'tuned-sm': { id: 'WorldGen-0.5B', url: '/models/WorldGen-0.5B/',                                     stockId: 'Qwen2.5-0.5B-Instruct-q4f16_1-MLC' }
};
```

Each tuned model reuses WebLLM's **stock** model lib for its size (`stockId` → that lib's WASM), so only
your fine-tuned weights download. Empty/absent map entry = that option is hidden in the picker.

---

## How it fits together

```
src/lib/llm-prompt.ts   grammar + schema + isValidOp + buildSystem/buildWorldState   ← single source of truth
        │  (imported by)
        ├── app runtime (WebLLM)         src/lib/llm.svelte.ts
        ├── dataset generator (Phase 1)  training/gen-dataset.ts
        └── eval battery                 tests/llm/{scenarios,crux}.ts
```

Add a new kind/zone/op? Update `kinds.ts`/`llm-prompt.ts`, add phrasings to `gen-dataset.ts`,
regenerate, retrain. The validator + battery keep everyone honest.

## Spotting what to train next

The app logs every tuned prompt → result to `prompts.log` (repo root, dev only). Scan it for
`(no valid ops)` / `ERROR` lines or wrong ops, add those phrasings to `gen-dataset.ts`, regenerate,
and retrain — a tight feedback loop from real usage to the next dataset.
