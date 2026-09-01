#!/usr/bin/env bash
# Equal-token matrix driver (mission §42): 4k -> 8k -> 16k -> 24k, then
# native-default (§43). Each run is a full 9-task x 4-variant codex matrix.
# Skips any budget whose result file already exists with the final
# 'summary' key (resumable after interruption).
set -u
cd "$(dirname "$0")"
export SCC_BIN="${SCC_BIN:-/Users/rocket/system_ir/target/debug/scc}"
OUT=/Users/rocket/system_ir/benchmarks/results
LABEL=codex
MODEL="gpt-5.2-codex"

run_one () {
  local budget="$1"; local out="$2"; shift 2
  if [ -f "$out" ] && python3 -c "import json,sys; sys.exit(0 if 'summary' in json.load(open('$out')) else 1)" 2>/dev/null; then
    echo "[driver] $out already complete — skipping"
    return 0
  fi
  echo "[driver] running budget=$budget out=$out"
  python3 run_write_matrix.py --budget "$budget" --agent-label "$LABEL" --model-label "$MODEL" --out "$out" "$@"
  echo "[driver] done budget=$budget rc=$?"
}

run_one 4000 "$OUT/write-matrix-4k.json"
run_one 8000 "$OUT/write-matrix-8k.json"
run_one 16000 "$OUT/write-matrix-16k.json"
run_one 24000 "$OUT/write-matrix-24k.json"
# Native-default (§20): no budget — each system at its product-native size.
if [ -f "$OUT/write-matrix-native-default.json" ] && python3 -c "import json,sys; sys.exit(0 if 'summary' in json.load(open('$OUT/write-matrix-native-default.json')) else 1)" 2>/dev/null; then
  echo "[driver] native-default already complete — skipping"
else
  echo "[driver] running native-default"
  python3 run_write_matrix.py --agent-label "$LABEL" --model-label "$MODEL" --out "$OUT/write-matrix-native-default.json"
  echo "[driver] done native-default rc=$?"
fi
echo "[driver] ALL MATRICES COMPLETE"
