#!/usr/bin/env bash
# PHASE 2b (optional, for EVAL) — convert the fused model to GGUF and drop it in .models/ so the
# existing battery scores it apples-to-apples vs the stock 1.5B/3B:  then run  `pnpm test:llm`.
# Requires a llama.cpp checkout (for convert_hf_to_gguf.py + llama-quantize).
set -euo pipefail
cd "$(dirname "$0")/.."   # repo root (so .models/ lands where the battery looks)

LLAMA_CPP="${LLAMA_CPP:-$HOME/llama.cpp}"
F16=.models/worldgen-1.5b-instruct-f16.gguf
QUANT=.models/worldgen-1.5b-instruct-q4_k_m.gguf

if [ ! -f "$LLAMA_CPP/convert_hf_to_gguf.py" ]; then
	echo "✗ llama.cpp not found at \$LLAMA_CPP=$LLAMA_CPP" >&2
	echo "  git clone https://github.com/ggml-org/llama.cpp && (cd llama.cpp && cmake -B build && cmake --build build -j)" >&2
	exit 1
fi

mkdir -p .models
python3 "$LLAMA_CPP/convert_hf_to_gguf.py" training/fused --outfile "$F16" --outtype f16
"$LLAMA_CPP/build/bin/llama-quantize" "$F16" "$QUANT" Q4_K_M 2>/dev/null || "$LLAMA_CPP/llama-quantize" "$F16" "$QUANT" Q4_K_M

echo "✓ GGUF → $QUANT"
echo "  now: pnpm test:llm   (the battery auto-detects worldgen-*.gguf in .models/)"
