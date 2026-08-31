#!/usr/bin/env python3
"""Scoped REAL writable coding-agent matrix.

Runs actual coding agents on real corpus tasks in WRITABLE mode: each
task gets an isolated disposable repo copy; the agent may edit; success
is decided by a behavioral EVALUATOR (benchmarks/evaluators/run.py),
never by the agent exit code and never by a comment-gamable grep.

Variants compared at the shared BUDGET (default 8000, chars/4):
  raw              — no context artifact
  aider-repomap    — pinned aider RepoMap
  repomix-compress  — pinned repomix --compress
  scc-atlas        — Atlas-only
  scc-surface      — Surface-only
  scc-full         — startup + task + Structural Source (`scc context
                      structural --task`), budget enforced on the FINAL
                      concatenation. Indexed against the disposable copy.

ALL variants go through the same writable runner (`run_write_task`).

HONESTY: this is a SCOPED matrix unless `--corpus` is passed. Live Codex /
Claude numbers are NOT checked in; run the documented command with credentials.
"""

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("rcb", HERE / "run_context_bench.py")
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)

WRITABLE_AGENT_CMD = (
    "codex exec --json --sandbox workspace-write --skip-git-repo-check "
    "--ephemeral --color never -C . -"
)

EVALUATOR = 'python3 "$SCC_EVALUATORS/run.py"'

# The scoped corpus: 6 tasks across 4 archetypes. Success is the
# behavioral evaluator (run.py) exiting 0 — never a grep.
SCOPED_TASKS = [
    {"repo": "http-service-python", "id": "http-service.rename-transcript-field",
     "goal": "rename the transcript field in the api response",
     "validate": f'{EVALUATOR} http-service.rename-transcript-field'},
    {"repo": "queue-worker-ts", "id": "queue-worker.asr-retry",
     "goal": "add retry handling to the asr transcription call",
     "validate": f'{EVALUATOR} queue-worker.asr-retry'},
    {"repo": "ts-api-web", "id": "ts-api-web.pagination",
     "goal": "add pagination to the users list endpoint",
     "validate": f'{EVALUATOR} ts-api-web.pagination'},
    {"repo": "py-queue-service", "id": "py-queue.empty-messages",
     "goal": "make the consumer tolerate empty messages",
     "validate": f'{EVALUATOR} py-queue.empty-messages'},
    {"repo": "http-service-python", "id": "http-service.health-check",
     "goal": "add a health check endpoint",
     "validate": f'{EVALUATOR} http-service.health-check'},
    {"repo": "queue-worker-ts", "id": "queue-worker.street-vocabulary",
     "goal": "change street name resolution to use department vocabulary",
     "validate": f'{EVALUATOR} queue-worker.street-vocabulary'},
]

BUDGET = h.DEFAULT_BUDGET
WRITABLE_VARIANTS = ("raw", "aider-repomap", "repomix-compress", "scc-atlas", "scc-surface", "scc-full")


def git_rev(path):
    try:
        r = subprocess.run(
            ["git", "-C", str(path), "rev-parse", "HEAD"],
            capture_output=True, text=True, timeout=15,
        )
        if r.returncode == 0:
            return r.stdout.strip()
    except Exception:
        pass
    return None


def infer_agent_name(agent_cmd):
    c = (agent_cmd or "").lower()
    if "claude" in c:
        return "claude"
    if "codex" in c:
        return "codex"
    tok = (agent_cmd or "unknown").split()[0]
    return Path(tok).name or "unknown"


def probe_agent_version(agent_cmd):
    exe = (agent_cmd or "").split()[0]
    if not exe:
        return None
    for flag in ("--version", "-V"):
        try:
            r = subprocess.run([exe, flag], capture_output=True, text=True, timeout=15)
            out = (r.stdout or r.stderr).strip().splitlines()
            if r.returncode == 0 and out:
                return out[0][:200]
        except Exception:
            continue
    return None


def collect_meta(agent_cmd, scc_bin, extra=None):
    meta = {
        "agent": infer_agent_name(agent_cmd),
        "agent_cmd": agent_cmd,
        "model": os.environ.get("SCC_BENCH_MODEL") or os.environ.get("OPENAI_MODEL") or os.environ.get("ANTHROPIC_MODEL"),
        "agent_version": probe_agent_version(agent_cmd),
        "scc_bin": scc_bin,
        "scc_revision": git_rev(h.ROOT),
        "harness_revision": git_rev(h.ROOT),
        "mode": "writable",
        "budget": BUDGET,
        "note": "scoped real-agent matrix unless --corpus; NOT fabricated scores",
        "live_command": (
            "python3 benchmarks/external/run_write_matrix.py "
            "--agent-cmd '<codex|claude writable cmd>' --scc-bin ./target/release/scc "
            "--out benchmarks/results/write-matrix-live.json"
        ),
    }
    if extra:
        meta.update(extra)
    return meta


def run_variant(variant, task, workdir, scc_bin=None, agent_cmd=None):
    """One (variant, task) cell through the SAME writable runner.

    Isolation, artifact construction (including scc-full Structural
    Source against the disposable copy), and the evaluator all live in
    `run_writable_variant` / `run_write_task`. Native and external variants
    must not take different runners.
    """
    cell_dir = Path(workdir) / f"{variant}--{task['id']}"
    cell_dir.mkdir(parents=True, exist_ok=True)
    grouped = {task["repo"]: [task]}
    rows, skipped = h.run_writable_variant(
        variant, grouped, BUDGET, agent_cmd, cell_dir, scc_bin=scc_bin)
    if skipped:
        return {"task_success": False, "run_completion": False,
                "context_tokens": 0, "wall_sec": 0.0, "error": skipped.get("error")}
    row = rows[0] if rows else {}
    return {
        "task_success": bool(row.get("tasks_passed")),
        "run_completion": bool(row.get("run_completion_rate")),
        "context_tokens": row.get("context_tokens", 0),
        "patch_produced": bool(row.get("patch_rate")),
        "modified_files": None,
        "wall_sec": row.get("mean_wall_sec", 0),
        "error": row.get("error"),
    }


def paired_or_none(a, b):
    if len(a) != len(b) or not a:
        return None
    mean, lo, hi = h.paired_bootstrap_ci(a, b)
    return {"mean_diff": mean, "ci95": [lo, hi],
            "note": ("CI crosses zero — no superiority claim" if lo <= 0 <= hi
                     else "CI excludes zero (scoped subset only)")}


def main(argv):
    import argparse
    parser = argparse.ArgumentParser(prog="run_write_matrix.py")
    parser.add_argument("--scc-bin", default=os.environ.get("SCC_BIN") or "scc")
    parser.add_argument("--agent-cmd", default=None,
                        help="writable agent command (default: codex workspace-write)")
    parser.add_argument("--out", default=str(HERE.parent / "results" / "write-matrix.json"))
    parser.add_argument("--tasks", help="comma-separated task ids (default: the scoped 6)")
    parser.add_argument("--corpus", action="store_true",
                        help="use the full evaluator-backed benchmarks/tasks.json corpus")
    parser.add_argument("--variants", default=",".join(WRITABLE_VARIANTS),
                        help="comma-separated variants")
    parser.add_argument("--skip-live", action="store_true",
                        help="do not invoke the agent; emit a skipped live-matrix stub")
    args = parser.parse_args(argv)

    agent_cmd = args.agent_cmd or WRITABLE_AGENT_CMD
    variants = tuple(v.strip() for v in args.variants.split(",") if v.strip())

    if args.corpus:
        loaded = h.load_tasks()
        tasks = []
        for repo, repo_tasks in loaded.items():
            for t in repo_tasks:
                if t.get("validate"):
                    tasks.append({"repo": repo, **t})
    else:
        tasks = SCOPED_TASKS
        if args.tasks:
            wanted = set(args.tasks.split(","))
            tasks = [t for t in SCOPED_TASKS if t["id"] in wanted]

    results = {
        "meta": collect_meta(agent_cmd, args.scc_bin),
        "cells": {},
    }

    if args.skip_live:
        results["meta"]["skipped"] = True
        results["meta"]["valid"] = False
        results["meta"]["invalid_reason"] = "live agent matrix skipped (no credentials / --skip-live)"
        results["summary"] = {"skipped": True, "n_tasks": len(tasks), "variants": list(variants)}
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(results, indent=2))
        print(json.dumps(results["summary"], indent=2))
        return 0

    workdir = Path(tempfile.mkdtemp(prefix="scc-write-matrix-"))
    try:
        for variant in variants:
            for task in tasks:
                key = f"{variant}/{task['id']}"
                print(f"[matrix] {key} ...", flush=True)
                cell = run_variant(variant, task, workdir, args.scc_bin, agent_cmd=agent_cmd)
                results["cells"][key] = cell
                print(f"          success={cell.get('task_success')} "
                      f"completion={cell.get('run_completion')} wall={cell.get('wall_sec')}s",
                      flush=True)
    finally:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(results, indent=2))

    ids = [t["id"] for t in tasks]

    def series(variant):
        vals = []
        n = 0
        for i in ids:
            cell = results["cells"].get(f"{variant}/{i}")
            if cell is None or cell.get("error"):
                continue
            n += 1
            vals.append(1.0 if cell.get("task_success") else 0.0)
        rate = (sum(vals) / n) if n else None
        return rate, n, vals

    rates = {v: series(v) for v in variants}
    skipped = {v: sum(1 for i in ids if results["cells"].get(f"{v}/{i}", {}).get("error"))
               for v in variants}

    scc_vals = rates.get("scc-full", (None, 0, []))[2]
    raw_vals = rates.get("raw", (None, 0, []))[2]
    aider_vals = rates.get("aider-repomap", (None, 0, []))[2]
    repomix_vals = rates.get("repomix-compress", (None, 0, []))[2]

    # micro = per-task; macro = per-repo mean
    by_repo = {}
    for t in tasks:
        by_repo.setdefault(t["repo"], []).append(t["id"])

    def macro(variant):
        repo_rates = []
        for repo, task_ids in by_repo.items():
            vals = []
            for i in task_ids:
                cell = results["cells"].get(f"{variant}/{i}")
                if cell is None or cell.get("error"):
                    continue
                vals.append(1.0 if cell.get("task_success") else 0.0)
            if vals:
                repo_rates.append(sum(vals) / len(vals))
        return (sum(repo_rates) / len(repo_rates)) if repo_rates else None

    summary = {
        "n_tasks": len(ids),
        "variants": list(variants),
        "micro_task_success": {v: rates[v][0] for v in variants},
        "macro_repo_success": {v: macro(v) for v in variants},
        "skipped_cells": skipped,
        "paired_ci_scc_minus_raw": paired_or_none(scc_vals, raw_vals),
        "paired_ci_scc_minus_aider": paired_or_none(scc_vals, aider_vals),
        "paired_ci_scc_minus_repomix": paired_or_none(scc_vals, repomix_vals),
    }
    results["summary"] = summary
    Path(args.out).write_text(json.dumps(results, indent=2))
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
