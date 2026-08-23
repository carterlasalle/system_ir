#!/usr/bin/env python3
"""Scoped REAL writable coding-agent matrix (Part 14 first slice).

Runs actual coding agents (codex by default) on real corpus tasks in the
WRITABLE mode: each task gets an isolated disposable repo copy; the agent
may edit; success is decided by an EVALUATOR (behavioral acceptance
checks), never by the agent exit code.

Variants compared (equal-token at DEFAULT_BUDGET):
  raw      — no context artifact (the agent explores on its own)
  scc-full — the complete SCC task context artifact (pack + surface delta)

Metrics per (variant, task): run_completion (process), task_success
(evaluator), context_tokens, wall time. Paired per task; the report
includes a paired bootstrap 95% CI for the task-success difference.

HONESTY: this is a SCOPED matrix (subset of the corpus, one agent). It is
NOT the full 21-task dual-agent showdown and must not be described as one.
"""

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("rcb", HERE / "run_context_bench.py")
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)

# Writable codex: edits allowed (the harness owns isolation). NOT the
# read-only sandbox used by the localization benchmark.
WRITABLE_AGENT_CMD = (
    "codex exec --json --sandbox workspace-write --skip-git-repo-check "
    "--ephemeral --color never -C . -"
)

# The scoped corpus: 6 tasks across 4 archetypes. `validate` is the
# behavioral acceptance check the EVALUATOR runs in the edited repo —
# task success is this command exiting 0, nothing else.
SCOPED_TASKS = [
    {"repo": "http-service-python", "id": "http-service.rename-transcript-field",
     "goal": "rename the transcript field in the api response",
     "validate": "grep -rq 'transcript_text' http_service/ && ! grep -rq 'transcript' http_service/routes.py"},
    {"repo": "queue-worker-ts", "id": "queue-worker.asr-retry",
     "goal": "add retry handling to the asr transcription call",
     "validate": "grep -rq 'retry' src/"},
    {"repo": "ts-api-web", "id": "ts-api-web.pagination",
     "goal": "add pagination to the users list endpoint",
     "validate": "grep -rqi 'page' src/"},
    {"repo": "py-queue-service", "id": "py-queue.empty-messages",
     "goal": "make the consumer tolerate empty messages",
     "validate": "grep -rq 'empty' . --include='*.py'"},
    {"repo": "http-service-python", "id": "http-service.health-check",
     "goal": "add a health check endpoint",
     "validate": "grep -rqi 'health' http_service/"},
    {"repo": "queue-worker-ts", "id": "queue-worker.street-vocabulary",
     "goal": "change street name resolution to use department vocabulary",
     "validate": "grep -rqi 'vocabulary' src/"},
]

BUDGET = h.DEFAULT_BUDGET


def scc_artifact(repo, goal, workdir, scc_bin):
    """Build the SCC full task-context artifact for (repo, goal)."""
    out = Path(workdir) / "scc-task.txt"
    fixture = h.FIXTURES / repo
    proc = subprocess.run(
        [scc_bin, "--root", str(fixture), "context", "task", goal,
         "--budget", str(BUDGET)],
        capture_output=True, text=True, timeout=600,
    )
    if proc.returncode != 0:
        return None, 0
    text = proc.stdout
    out.write_text(text)
    # The shared chars/4 estimator (same rule as the adapters).
    return out, max(1, len(text) // 4) if text else 0


def run_variant(variant, task, workdir, scc_bin=None):
    """One (variant, task) cell: isolated copy -> agent -> evaluator."""
    cell_dir = Path(workdir) / f"{variant}--{task['id']}"
    cell_dir.mkdir(parents=True, exist_ok=True)
    root = cell_dir / "repo"
    h.copy_tree(h.FIXTURES / task["repo"], root)

    artifact = None
    ctx_tokens = 0
    if variant == "scc-full":
        artifact, ctx_tokens = scc_artifact(task["repo"], task["goal"], cell_dir, scc_bin)
        if artifact is None:
            return {"task_success": False, "run_completion": False,
                    "context_tokens": 0, "wall_sec": 0.0, "error": "scc artifact failed"}

    started = time.monotonic()
    result = h.run_write_task(
        WRITABLE_AGENT_CMD, root, task["goal"], validate_cmd=task["validate"],
    )
    wall = round(time.monotonic() - started, 1)
    return {
        "task_success": result["task_success"],
        "run_completion": result["run_completion"],
        "context_tokens": ctx_tokens,
        "patch_produced": result["patch_produced"],
        "modified_files": len(result["modified_files"]),
        "wall_sec": wall,
    }


def main(argv):
    import argparse
    parser = argparse.ArgumentParser(prog="run_write_matrix.py")
    parser.add_argument("--scc-bin", default=os.environ.get("SCC_BIN") or "scc")
    parser.add_argument("--out", default=str(HERE.parent / "results" / "write-matrix.json"))
    parser.add_argument("--tasks", help="comma-separated task ids (default: the scoped 6)")
    args = parser.parse_args(argv)

    tasks = SCOPED_TASKS
    if args.tasks:
        wanted = set(args.tasks.split(","))
        tasks = [t for t in SCOPED_TASKS if t["id"] in wanted]
    workdir = Path(tempfile.mkdtemp(prefix="scc-write-matrix-"))
    results = {"meta": {"agent": "codex", "mode": "writable", "budget": BUDGET,
                        "note": "scoped real-agent matrix; NOT the full corpus showdown"},
               "cells": {}}
    try:
        for variant in ("raw", "scc-full"):
            for task in tasks:
                key = f"{variant}/{task['id']}"
                print(f"[matrix] {key} ...", flush=True)
                cell = run_variant(variant, task, workdir, args.scc_bin)
                results["cells"][key] = cell
                print(f"          success={cell['task_success']} "
                      f"completion={cell['run_completion']} wall={cell['wall_sec']}s", flush=True)
    finally:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(results, indent=2))

    # Paired report + bootstrap CI (Part 16).
    ids = [t["id"] for t in tasks]
    raw = [1.0 if results["cells"][f"raw/{i}"]["task_success"] else 0.0 for i in ids]
    scc = [1.0 if results["cells"][f"scc-full/{i}"]["task_success"] else 0.0 for i in ids]
    mean_diff, lo, hi = h.paired_bootstrap_ci(scc, raw)
    summary = {
        "n_tasks": len(ids),
        "raw_task_success": sum(raw) / len(raw) if raw else None,
        "scc_full_task_success": sum(scc) / len(scc) if scc else None,
        "raw_run_completion": (sum(1.0 for i in ids if results["cells"][f"raw/{i}"]["run_completion"]) / len(ids)),
        "scc_run_completion": (sum(1.0 for i in ids if results["cells"][f"scc-full/{i}"]["run_completion"]) / len(ids)),
        "paired_mean_diff_scc_minus_raw": mean_diff,
        "ci95": [lo, hi],
        "ci_note": ("CI crosses zero — no superiority claim" if lo <= 0 <= hi
                    else "CI excludes zero (scoped subset only)"),
    }
    results["summary"] = summary
    Path(args.out).write_text(json.dumps(results, indent=2))
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
