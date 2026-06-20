#!/usr/bin/env bash
# Runs INSIDE a fresh pinned python:3.11 (linux/amd64) container — invoked by convert-mlc.sh, not by
# hand. Converts the fused HF model at $FUSED → $OUT: MLC q4f16_1 weight shards + tensor-cache.json +
# the runtime mlc-chat-config.json + tokenizer files.
#
# Why this shape: MLC's from-source TVM never registers its LLVM codegen on arm64 (dead end), so we use
# the PREBUILT x86_64 tvm wheel (codegen works) + mlc_llm's *python* from source. That wheel ships no
# compiled libmlc_llm.so, so a few import paths convert_weight/gen_config never touch (serve engine,
# tvm.contrib target tools, the C++ mlc.Tokenizer) must be made optional — that's mlc-prebuilt.patch.
set -euo pipefail
: "${FUSED:?set FUSED (path to fused HF model, in-container)}"
: "${OUT:?set OUT (output dir, in-container)}"
MLC_AI_VER="${MLC_AI_VER:-0.26.dev61}"
MLC_LLM_COMMIT="${MLC_LLM_COMMIT:-2008fe8}"

apt-get update -qq && apt-get install -y -qq git >/dev/null
pip install -q --pre -f https://mlc.ai/wheels "mlc-ai-nightly-cpu==$MLC_AI_VER"
pip install -q tqdm pydantic safetensors transformers
pip install -q torch --index-url https://download.pytorch.org/whl/cpu   # safetensors loader needs torch

[ -d /mlc-llm ] || git clone -q https://github.com/mlc-ai/mlc-llm /mlc-llm
cd /mlc-llm
git checkout -q "$MLC_LLM_COMMIT"
git checkout -q -- .                       # drop any prior patch so re-runs apply cleanly
git apply /work/training/mlc-prebuilt.patch
export PYTHONPATH=/mlc-llm/python

rm -rf "$OUT" && mkdir -p "$OUT"
python -m mlc_llm convert_weight "$FUSED" --quantization q4f16_1 -o "$OUT"
# gen_config also copies tokenizer.json + tokenizer_config.json into $OUT
python -m mlc_llm gen_config "$FUSED" --quantization q4f16_1 --conv-template qwen2 \
	--context-window-size 1024 --prefill-chunk-size 1024 -o "$OUT"
echo "✓ convert_weight + gen_config → $OUT"
