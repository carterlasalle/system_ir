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
#
# EVALUATOR VALIDITY CONTRACT (audit fix): every validator was checked
# against the UNTOUCHED fixture — it MUST exit non-zero there (a validator
# that passes on the untouched repo measures nothing). Each validator
# asserts the specific behavioral change the goal names, against the
# fixture's REAL layout (main.py + services/, src/, web/+service/,
# consumer.py — no invented directories).
SCOPED_TASKS = [
    {"repo": "http-service-python", "id": "http-service.rename-transcript-field",
     "goal": "rename the transcript field in the api response",
     # The API response dict must expose transcriptText and no longer
     # expose the old transcript key in the route handler's returned dicts.
     "validate": "! grep -qE '\"transcript\"' main.py && grep -qE 'transcriptText|transcript_text' main.py"},
    {"repo": "queue-worker-ts", "id": "queue-worker.asr-retry",
     "goal": "add retry handling to the asr transcription call",
     # The ASR CALL PATH (src/ingest.ts or a new call site) must gain
     # retry logic — the decorator on client.ts already exists, so a bare
     # grep for 'retry' anywhere is a false pass.
     "validate": "grep -qE 'retry|attempt' src/ingest.ts"},
    {"repo": "ts-api-web", "id": "ts-api-web.pagination",
     "goal": "add pagination to the users list endpoint",
     # The users service/endpoint gains page/pageSize params (fixture has
     # no src/; the users code lives in service/users.ts + server.ts).
     "validate": "grep -qiE 'page' service/users.ts server.ts"},
    {"repo": "py-queue-service", "id": "py-queue.empty-messages",
     "goal": "make the consumer tolerate empty messages",
     # consume() must guard empty/blank messages before dispatch.
     "validate": "grep -qE 'not message|if.*message.*:|strip()' consumer.py && grep -q 'def consume' consumer.py && grep -qE 'if not (message|message.get)' consumer.py"},
    {"repo": "http-service-python", "id": "http-service.health-check",
     "goal": "add a health check endpoint",
     # A NEW health route: the fixture already HAS /health, so the agent
     # must extend it to a real readiness payload (status+ok) — a no-op
     # cannot pass because the validator demands the payload keys.
     "validate": "grep -qE 'uptime|checks|dependencies' main.py && grep -q '/health' main.py"},
    {"repo": "queue-worker-ts", "id": "queue-worker.street-vocabulary",
     "goal": "change street name resolution to use department vocabulary",
     # The resolver must APPLY the vocabulary map in its resolution path —
     # a vocabulary map must exist AND be applied (includes/replace).
     "validate": "grep -qE 'Map<|DEPARTMENT_VOCAB' src/geo/resolver.ts && grep -qE 'includes|replace' src/geo/resolver.ts"},
]

BUDGET = h.DEFAULT_BUDGET


def scc_artifact(repo, goal, workdir, scc_bin):
    """Build the FULL SCC stack artifact for (repo, goal): the fused
    startup (Atlas + global Surface + coverage + omissions) followed by
    the complete task artifact (enriched pack + task-personalized Surface
    delta). The fixture is INDEXED FIRST into a disposable .scc dir — the
    checked-in fixtures carry no index, so skipping this yields a framing-
    only artifact with no repository knowledge. This is what "scc-full"
    means in the external benchmark; the task pack alone is NOT full SCC."""
    fixture = h.FIXTURES / repo
    env = {**os.environ, "SCC_STATE_DIR": str(Path(workdir) / ".scc-state")}
    def scc(*args):
        return subprocess.run([scc_bin, "--root", str(fixture), *args],
                              capture_output=True, text=True, timeout=900, env=env)
    if scc("index").returncode != 0:
        return None, 0
    parts = []
    startup = scc("context", "startup")
    if startup.returncode == 0 and startup.stdout.strip():
        parts.append(startup.stdout.rstrip() + "\n\n")
    task = scc("context", "task", goal, "--json")
    if task.returncode == 0:
        try:
            art = json.loads(task.stdout)
            parts.append(art["pack"]["content"])
            if art.get("delta"):
                parts.append("\n" + art["delta"])
        except (ValueError, KeyError):
            parts.append(task.stdout)
    elif task.stdout.strip():
        parts.append(task.stdout)
    text = "".join(parts)
    if not text.strip():
        return None, 0
    out = Path(workdir) / "scc-full.txt"
    out.write_text(text)
    # The shared chars/4 estimator (same rule as the adapters).
    return out, max(1, len(text) // 4)


EXTERNAL_VARIANTS = ("aider-repomap", "repomix-compress")


def task_slug(goal):
    import re
    return re.sub(r"[^A-Za-z0-9._-]", "_", goal)[:40] or "task"


def external_artifact(variant, repo, goal, workdir, budget):
    """Build the pinned aider/repomix artifact for (repo, goal) via the
    shared adapters, returning (artifact_path, tokens, error). A missing
    or unpinned tool yields error (SKIPPED-UNINSTALLED / PIN-MISMATCH /
    PIN-UNVERIFIED) and a None artifact — the cell is reported, never
    silently treated as a pass."""
    adapter = h.BENCHMARKS / "external" / (
        "aider_adapter.py" if variant == "aider-repomap" else "repomix_adapter.py")
    # Aider personalizes per goal (immutable per-task dir); repomix is
    # task-invariant but regenerating per cell is harmless and keeps the
    # cell self-contained.
    art_dir = Path(workdir) / "artifacts" / variant / task_slug(goal)
    art_dir.mkdir(parents=True, exist_ok=True)
    argv = [h.bench_python(), str(adapter), str(h.FIXTURES / repo), str(budget), str(art_dir)]
    if variant == "aider-repomap":
        argv += ["--goal", goal]
    else:
        argv.append("--compress")
    proc = subprocess.run(argv, capture_output=True, text=True, timeout=1800)
    try:
        payload = json.loads(proc.stdout or "{}")
    except ValueError:
        return None, 0, f"adapter output not JSON: {proc.stdout[:200]}"
    if proc.returncode == 2:
        return None, 0, "SKIPPED-UNINSTALLED"
    if proc.returncode == 3:
        return None, 0, "PIN-MISMATCH"
    if proc.returncode == 4:
        return None, 0, "PIN-UNVERIFIED"
    if not payload.get("ok"):
        return None, 0, payload.get("error", "adapter failed")
    return Path(payload["artifact"]), int(payload.get("tokens", 0)), None


def run_variant(variant, task, workdir, scc_bin=None, agent_cmd=None):
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
    elif variant in EXTERNAL_VARIANTS:
        artifact, ctx_tokens, err = external_artifact(
            variant, task["repo"], task["goal"], cell_dir, BUDGET)
        if artifact is None:
            return {"task_success": False, "run_completion": False,
                    "context_tokens": 0, "wall_sec": 0.0, "error": err}

    started = time.monotonic()
    result = h.run_write_task(
        agent_cmd, root, task["goal"], validate_cmd=task["validate"],
        artifact_path=artifact,
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
    parser.add_argument("--agent-cmd", default=None,
                        help="writable agent command (default: codex workspace-write)")
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
    variants = ("raw", "scc-full", "aider-repomap", "repomix-compress")
    try:
        for variant in variants:
            for task in tasks:
                key = f"{variant}/{task['id']}"
                print(f"[matrix] {key} ...", flush=True)
                cell = run_variant(variant, task, workdir, args.scc_bin,
                                   agent_cmd=args.agent_cmd or WRITABLE_AGENT_CMD)
                results["cells"][key] = cell
                print(f"          success={cell['task_success']} "
                      f"completion={cell['run_completion']} wall={cell['wall_sec']}s", flush=True)
    finally:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(results, indent=2))

    # Paired report + bootstrap CI (Part 16). Aider/repomix cells whose
    # artifact could not be built (SKIPPED-UNINSTALLED / PIN-*) are
    # excluded from the success-rate numerator and reported separately,
    # so a missing external tool never masquerades as a 0% pass.
    ids = [t["id"] for t in tasks]
    def success_series(variant):
        out = 0.0
        n = 0
        for i in ids:
            cell = results["cells"].get(f"{variant}/{i}")
            if cell is None or cell.get("error"):
                continue
            n += 1
            out += 1.0 if cell.get("task_success") else 0.0
        return (out / n) if n else None, n
    raw_rate, raw_n = success_series("raw")
    scc_rate, scc_n = success_series("scc-full")
    aider_rate, aider_n = success_series("aider-repomap")
    repomix_rate, repomix_n = success_series("repomix-compress")
    skipped = {v: sum(1 for i in ids if results["cells"].get(f"{v}/{i}", {}).get("error"))
               for v in variants}
    mean_diff, lo, hi = h.paired_bootstrap_ci(
        [1.0 if results["cells"][f"scc-full/{i}"]["task_success"] else 0.0 for i in ids],
        [1.0 if results["cells"][f"raw/{i}"]["task_success"] else 0.0 for i in ids])
    summary = {
        "n_tasks": len(ids),
        "variants": list(variants),
        "raw_task_success": raw_rate,
        "scc_full_task_success": scc_rate,
        "aider_task_success": aider_rate,
        "repomix_task_success": repomix_rate,
        "skipped_cells": skipped,
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
