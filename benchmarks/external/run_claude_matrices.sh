#!/usr/bin/env bash
# Claude matrix chain: 8k -> 16k -> 24k -> native-default (mission §42-§43
# second agent). Resumable: skips files with a completed summary.
set -u
cd "$(dirname "$0")"
export SCC_BIN="${SCC_BIN:-/Users/rocket/system_ir/target/debug/scc}"
OUT=/Users/rocket/system_ir/benchmarks/results
LABEL=claude
MODEL=claude-sonnet-4-5
CMD="claude -p --model claude-sonnet-4-5 --dangerously-skip-permissions"

done_p () { [ -f "$1" ] && python3 -c "import json,sys; d=json.load(open('$1')); sys.exit(0 if ('summary' in d and sum(1 for c in d.get('cells',{}).values() if not c.get('run_completion'))==0) else 1)" 2>/dev/null; }

for b in 8000 16000 24000; do
  f="$OUT/write-matrix-$((b/1000))k-claude.json"
  if done_p "$f"; then echo "[driver] $f complete — skip"; continue; fi
  echo "[driver] claude budget=$b"
  python3 run_write_matrix.py --budget "$b" --agent-cmd "$CMD" --agent-label "$LABEL" --model-label "$MODEL" --out "$f"
  echo "[driver] done budget=$b rc=$?"
done
f="$OUT/write-matrix-native-default-claude.json"
if ! done_p "$f"; then
  echo "[driver] claude native-default"
  python3 run_write_matrix.py --agent-cmd "$CMD" --agent-label "$LABEL" --model-label "$MODEL" --out "$f"
  echo "[driver] done native-default rc=$?"
fi
echo "[driver] CLAUDE MATRICES COMPLETE"
