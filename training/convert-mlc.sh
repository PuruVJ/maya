#!/usr/bin/env bash
# PHASE 3 — convert a fused HF model (./fuse.sh output) → WebLLM-ready MLC weights, reproducibly, in a
# pinned x86_64 Docker container. No local C++ build (see _mlc_convert.sh for why + the pinned versions).
#
#   ./fuse.sh && ./convert-mlc.sh                              # 1.5B → static/models/WorldGen-1.5B
#   FUSED=./fused-0.5b NAME=WorldGen-0.5B ./convert-mlc.sh     # 0.5B variant
#
# Requires Docker with linux/amd64 emulation (OrbStack / Docker Desktop). Output lands in
# training/dist/<NAME> and is published to static/models/<NAME> for local dev. Both are gitignored
# (839 MB / 277 MB). For prod, upload training/dist/<NAME> to a Hugging Face repo and point the matching
# TUNED[...].url in src/lib/llm.svelte.ts at its /resolve/main/ URL.
set -euo pipefail
cd "$(dirname "$0")"                       # training/

FUSED="${FUSED:-./fused}"
NAME="${NAME:-WorldGen-1.5B}"
OUT="./dist/$NAME"

[ -d "$FUSED" ] || { echo "✗ $FUSED missing — run ./fuse.sh first" >&2; exit 1; }

echo "▶ converting $FUSED → MLC q4f16_1 ($NAME) in a pinned x86_64 container…"
docker run --rm --platform linux/amd64 -v "$PWD/..":/work -w /work \
	-e FUSED="/work/training/${FUSED#./}" -e OUT="/work/training/dist/$NAME" \
	python:3.11 bash /work/training/_mlc_convert.sh

# WebLLM rewrites the model URL HF-style (…/resolve/main/<file>) — mirror that layout (hardlinks, 0 disk)
mkdir -p "$OUT/resolve/main"
( cd "$OUT" && for f in *.json *.bin; do ln -f "$f" "resolve/main/$f"; done )

# publish to static/ for local dev serving
mkdir -p ../static/models && rm -rf "../static/models/$NAME" && cp -r "$OUT" "../static/models/$NAME"

echo "✓ $NAME → static/models/$NAME"
echo "  Wire it in src/lib/llm.svelte.ts (TUNED map: id/url/stockId), then pick it in the model picker."
