#!/usr/bin/env bash
# TraceLayer hook adapter for Pi (earendil-works/pi).
# Usage: trace-hook.sh <event> [pi]
# Reads the Pi hook JSON payload from stdin, runs the deterministic trace
# hook, and emits the Pi permission contract on stdout.
set -uo pipefail

EVENT="${1:-}"
[ -n "$EVENT" ] || { echo "usage: $0 <event> [pi]" >&2; exit 2; }

PAYLOAD="$(cat 2>/dev/null || true)"
if [ -z "$PAYLOAD" ] && [ -n "${CLAUDE_TOOL_INPUT:-}" ]; then
  PAYLOAD="$CLAUDE_TOOL_INPUT"
fi

# Normalize harness payload: derive `path` for edit/write tools and pin the
# session id so block-once state is stable across tool calls.
PAYLOAD="$(
  python3 - "$PAYLOAD" <<'PY'
import json, os, sys
raw = sys.argv[1]
if not raw:
    raw = "{}"
d = json.loads(raw)
if d.get("hook_event_name") in ("PreToolUse", "PostToolUse") and "path" not in d:
    ti = d.get("tool_input") or {}
    for key in ("path", "file_path", "filePath"):
        if ti.get(key):
            d["path"] = ti[key]
            break
    if "path" not in d:
        files = os.environ.get("CLAUDE_FILE_PATHS", "").split()
        if files:
            d["path"] = files[0]
d.setdefault("session_id", os.environ.get("TRACE_SESSION", "default"))
print(json.dumps(d))
PY
)"

OUT="$(printf '%s' "$PAYLOAD" | uv run trace hook "$EVENT" --format json 2>/dev/null)"
RC=$?

if [ "$RC" -eq 1 ]; then
  REASON="$(printf '%s' "$OUT" | python3 -c '
import json, sys
try:
    print(json.load(sys.stdin).get("output", "blocked by trace policy"))
except Exception:
    print("blocked by trace policy")
')"
  REASON_JSON="$(python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))' <<<"$REASON")"
  printf '{"permissionDecision":"deny","permissionDecisionReason":%s}' "$REASON_JSON"
  exit 0
fi

# Allowed: surface bounded hook output (session-start, prompt-context,
# post-mutation, stop confirmation) to the harness.
printf '%s' "$OUT" | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
    print(d.get("output", ""), end="")
except Exception:
    pass'
exit 0
