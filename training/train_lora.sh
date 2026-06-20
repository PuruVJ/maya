#!/usr/bin/env bash
# PHASE 2a — LoRA fine-tune the world-builder on Apple Silicon (MLX). Run from anywhere; cd's to here.
# Trains adapters on training/data/{train,valid}.jsonl (generate them first: `pnpm gen:dataset`).
#
#   ./train_lora.sh                 # defaults (Qwen2.5-1.5B, 2500 iters ≈ ~2 epochs over 5k)
#   ITERS=4000 ./train_lora.sh      # more passes
#   BASE=Qwen/Qwen2.5-0.5B-Instruct ./train_lora.sh   # even lighter base
#
# M4 Max: 1.5B LoRA @ seq 4096, batch 4 is comfortable (well under unified memory); ~20–40 min.
set -euo pipefail
cd "$(dirname "$0")"

BASE="${BASE:-Qwen/Qwen2.5-1.5B-Instruct}"
ITERS="${ITERS:-1000}"   # compact prompt → fast convergence; pick the best val checkpoint
ADAPTERS="${ADAPTERS:-./adapters}"   # set per-base so variants don't collide (e.g. ./adapters-0.5b)

if [ ! -f ./data/train.jsonl ]; then
	echo "✗ training/data/train.jsonl missing — run \`pnpm gen:dataset\` first." >&2
	exit 1
fi

# `mlx_lm.lora` is the entry point; on newer mlx-lm use `mlx_lm lora` (same flags).
mlx_lm.lora \
	--model "$BASE" \
	--train \
	--data ./data \
	--adapter-path "$ADAPTERS" \
	--mask-prompt \
	--batch-size 8 \
	--num-layers 16 \
	--iters "$ITERS" \
	--learning-rate 1e-4 \
	--max-seq-length 768 \
	--steps-per-report 25 \
	--steps-per-eval 150 \
	--save-every 250

echo "✓ LoRA adapters → $ADAPTERS   (next: ./fuse.sh)"
