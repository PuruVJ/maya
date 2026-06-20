#!/usr/bin/env bash
# PHASE 2b — fuse the LoRA adapters back into the base weights → a standalone HF-format model that the
# MLC converter (convert-mlc.sh) consumes. Parameterized so 1.5B / 0.5B variants don't collide:
#   ./fuse.sh                                                              # 1.5B → training/fused
#   BASE=Qwen/Qwen2.5-0.5B-Instruct ADAPTERS=./adapters-0.5b FUSED=./fused-0.5b ./fuse.sh
set -euo pipefail
cd "$(dirname "$0")"

BASE="${BASE:-Qwen/Qwen2.5-1.5B-Instruct}"
ADAPTERS="${ADAPTERS:-./adapters}"
FUSED="${FUSED:-./fused}"

if [ ! -d "$ADAPTERS" ]; then
	echo "✗ $ADAPTERS missing — run ./train_lora.sh first." >&2
	exit 1
fi

mlx_lm.fuse --model "$BASE" --adapter-path "$ADAPTERS" --save-path "$FUSED"

echo "✓ fused HF model → $FUSED   (next: ./convert-mlc.sh)"
