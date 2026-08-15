#!/usr/bin/env python3
"""Deterministic mock agent for the Wave-15 external benchmark showdown.

The benchagent protocol pipes `SCC CONTEXT: <artifact>\n\nTASK: <goal>` to
the agent command on stdin ($SCC_GOAL carries the goal). This mock reads
stdin, extracts the repo-relative source paths the ARTIFACT mentions, and
emits a JSONL event stream (the benchagent protocol): a plan naming the
first artifact-surfaced path, one search, then reads of the surfaced paths.

Using the SAME mock agent command for every variant (SCC-native and
external-tool alike) is what makes the showdown equal-conditions: the only
input that varies between variants is the context artifact itself, so the
metric columns (first-plan accuracy, files opened, wrong-first locations,
context tokens) reflect the artifact, not the agent. Deterministic: no
network, no randomness, same input -> same output.

Usage:
    python3 mock_agent.py < <prompt>        (SCC_GOAL in the environment)
"""
# trace:exempt reason=internal-detail  # deterministic benchagent mock (external-bench suite)

import json
import os
import re
import sys

SOURCE_EXT_RE = re.compile(
    r"\b([A-Za-z0-9_./-]+\.(?:py|pyi|ts|tsx|js|jsx|mjs|cjs|rs|go|java|kt|kts|rb|php|c|h|cpp|hpp|cc|cs|dart|proto|txt|json|toml|yaml|yml|md|sh|sql|html|css|vue|svelte|swift|lua|xml|gradle|dockerfile))\b"
)


def main():
    text = sys.stdin.read()
    paths = []
    seen = set()
    for m in SOURCE_EXT_RE.finditer(text):
        p = m.group(1)
        if p not in seen:
            seen.add(p)
            paths.append(p)
    plan = paths[0] if paths else "scan"
    goal = os.environ.get("SCC_GOAL", "")

    def event(item):
        return json.dumps({"type": "item.completed", "item": item})

    sys.stdout.write(event({"type": "agent_message", "text": f"Plan: {plan}"}) + "\n")
    sys.stdout.write(
        event(
            {
                "type": "command_execution",
                "command": "/bin/zsh -lc \"rg -n .\"",
                "exit_code": 0,
            }
        )
        + "\n"
    )
    for p in paths[:6]:
        sys.stdout.write(
            event(
                {
                    "type": "mcp_tool_call",
                    "tool": "read_file",
                    "arguments": {"file_path": p},
                }
            )
            + "\n"
        )
    if goal:
        sys.stdout.write(event({"type": "agent_message", "text": f"TASK: {goal}"}) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
