#!/usr/bin/env bash
# §56 agent benchmark runner (Wave 8): compares Codex with and without SCC
# context over the 21-task fixture corpus via the `scc bench agent` harness.
#
# Usage:
#   benchmarks/run_agent_bench.sh [variant...]     # default: A E F
#   benchmarks/run_agent_bench.sh A                # baseline only
#
# Variants:
#   A  plain Codex (no SCC context)
#   E  Codex + full startup System Atlas (scc atlas)
#   F  Codex + atlas with lazy semantic resolution (scc atlas --resolve)
#
# The harness measures: wall time, exit status, output size, and
# localization = ground-truth files surfaced in the agent's output.
# Results print per variant; JSON summaries land in benchmarks/results/.

set -euo pipefail
cd "$(dirname "$0")/.."
SCC_BIN="${SCC_BIN:-$PWD/target/release/scc}"
[ -x "$SCC_BIN" ] || { echo "build first: cargo build --release -p scc-cli"; exit 1; }
command -v codex >/dev/null 2>&1 || { echo "codex CLI required"; exit 1; }
mkdir -p benchmarks/results

TIME_BUDGET=300
export PATH="$PWD/target/release:$PATH"

baseline_cmd='printf "%s\n" "$SCC_GOAL" | gtimeout '"$TIME_BUDGET"' codex exec --sandbox read-only --skip-git-repo-check --ephemeral --color never -C . - 2>/dev/null'

atlas_cmd='ATLAS=$("'"$SCC_BIN"'" atlas 2>/dev/null || true); printf "SCC SYSTEM ATLAS (startup architecture):\n%s\n\nTASK: %s\n" "$ATLAS" "$SCC_GOAL" | gtimeout '"$TIME_BUDGET"' codex exec --sandbox read-only --skip-git-repo-check --ephemeral --color never -C . - 2>/dev/null'

atlas_resolve_cmd='ATLAS=$("'"$SCC_BIN"'" atlas --resolve 2>/dev/null || true); printf "SCC SYSTEM ATLAS (startup architecture):\n%s\n\nTASK: %s\n" "$ATLAS" "$SCC_GOAL" | gtimeout '"$TIME_BUDGET"' codex exec --sandbox read-only --skip-git-repo-check --ephemeral --color never -C . - 2>/dev/null'

run_variant() {
  local v="$1" cmd="$2"
  echo "=== variant $v: 21 tasks through codex ==="
  "$SCC_BIN" bench agent --cmd "$cmd" 2>&1 | tee "benchmarks/results/agent-$v.txt"
}

variants=("$@")
[ "${#variants[@]}" -eq 0 ] && variants=(A E F)

for v in "${variants[@]}"; do
  case "$v" in
    A) run_variant A "$baseline_cmd" ;;
    E) run_variant E "$atlas_cmd" ;;
    F) run_variant F "$atlas_resolve_cmd" ;;
    *) echo "unknown variant $v (A|E|F)"; exit 1 ;;
  esac
done
echo "done — results in benchmarks/results/agent-{A,E,F}.txt"
