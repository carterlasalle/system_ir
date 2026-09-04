#!/usr/bin/env python3
"""Scoped REAL writable coding-agent matrix.

One writable runner for every variant (mission §21): isolated repo copy ->
context generation -> SAME agent -> SAME evaluator -> SAME metrics. Variants:
raw / scc-full / aider-repomap / repomix-compress.

Task corpus: derived from benchmarks/tasks.json by task id (§28) — one
canonical source; the evaluators live in evaluators.py and satisfy the
four-way meta-contract (§25: untouched FAIL, comment-only FAIL,
plausible-wrong FAIL, known-good PASS — enforced by test_evaluators.py in
CI before any experiment runs).

Metadata (§33): every cell records agent label/command/version, model,
SCC/bench/harness commits, corpus hash, mode, requested budget, actual
context tokens, per-task outcome and error TYPE (§32: agent-completed-
incorrectly vs infrastructure failure are never conflated).
"""
import hashlib
import importlib.util
import json
import os
import shutil
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

# Canonical corpus (§28): the writable tasks are the canonical tasks.json
# ids that have REGISTERED evaluators. SCOPED_TASKS is derived, never a
# divergent copy. `validate` shell snippets are GONE: the registry
# evaluators (behavioral where the fixture runs; explicitly classified
# structural otherwise) are the single source of acceptance truth.
import evaluators as _ev

def canonical_writable_tasks():
    tasks = []
    for t in _ev.load_tasks():
        if t["id"] in _ev.EVALUATORS:
            tasks.append({"repo": t["repo"], "id": t["id"], "goal": t["goal"],
                          "structural": _ev.is_structural(t["id"])})
    return tasks

SCOPED_TASKS = canonical_writable_tasks()

BUDGET = h.DEFAULT_BUDGET


NATIVE_SCC_VARIANTS = ("scc-full", "scc-atlas", "scc-surface", "scc-atlas-surface")


def scc_artifact(repo, goal, workdir, scc_bin, budget=None, variant="scc-full"):
    """Build an SCC artifact through the AUTHORITATIVE CLI builder
    (`scc bench external --artifact-only`, mission §18/§40). `scc-full` is
    the headline variant: fused startup (Atlas + Surface) + task pack +
    goal-selected Structural Source, with the FINAL-artifact budget
    enforced by the CLI (startup N/2, task N/4, structural remainder, hard
    shrink). Ablations (`scc-atlas`, `scc-surface`, `scc-atlas-surface`)
    are opt-in via --variants for secondary analysis. The harness never
    reconstructs SCC semantics in Python. Returns (artifact_path, tokens);
    raises RuntimeError on failure (infra error)."""
    out_dir = Path(workdir) / "artifacts" / variant
    out_dir.mkdir(parents=True, exist_ok=True)
    argv = [scc_bin, "bench", "external",
            "--variant", variant, "--repo", repo,
            "--artifact-only", goal, "--workdir", str(out_dir)]
    if budget is not None:
        argv += ["--budget", str(budget)]
    proc = subprocess.run(argv, capture_output=True, text=True, timeout=900)
    if proc.returncode != 0:
        raise RuntimeError(f"scc artifact builder failed: {(proc.stderr or proc.stdout)[:300]}")
    payload = json.loads(proc.stdout)
    tokens = int(payload.get("tokens", 0))
    # §19 postcondition: the shared estimator on the FINAL artifact must
    # be within the requested budget (the CLI enforces it; assert here so
    # a regression is loud, not silent).
    if budget is not None:
        text = Path(payload["artifact"]).read_text()
        actual = max(1, len(text) // 4)
        assert actual <= budget, f"equal-token violation: artifact {actual} > budget {budget}"
        assert tokens == actual or tokens <= budget, (tokens, actual, budget)
    return Path(payload["artifact"]), tokens


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
        return None, 0, f"SKIPPED-UNINSTALLED: {payload.get('error', '')[:200]}"
    if proc.returncode == 3:
        return None, 0, f"PIN-MISMATCH: {payload.get('error', '')[:200]}"
    if proc.returncode == 4:
        return None, 0, f"PIN-UNVERIFIED: {payload.get('error', '')[:200]}"
    if not payload.get("ok"):
        return None, 0, payload.get("error", "adapter failed")
    return Path(payload["artifact"]), int(payload.get("tokens", 0)), None


def run_variant(variant, task, workdir, scc_bin=None, agent_cmd=None,
                agent_label=None, model_label=None, budget=None):
    """One (variant, task) cell: isolated copy -> agent -> evaluator.

    Error typing (§32): `error` is set ONLY for infrastructure failures
    (artifact generation, missing tool); an agent that completed the task
    incorrectly has task_success=False and error=None. Infra-failed cells
    are excluded from success-rate numerators AND from the paired sets
    (reported separately, never masquerading as a coding failure).
    """
    cell_dir = Path(workdir) / f"{variant}--{task['id']}"
    cell_dir.mkdir(parents=True, exist_ok=True)
    root = cell_dir / "repo"
    h.copy_tree(h.FIXTURES / task["repo"], root)

    artifact = None
    ctx_tokens = 0
    error = None
    if variant in NATIVE_SCC_VARIANTS:
        try:
            artifact, ctx_tokens = scc_artifact(
                task["repo"], task["goal"], cell_dir, scc_bin, budget=budget,
                variant=variant)
        except (RuntimeError, AssertionError, ValueError) as exc:
            return {"task_success": False, "run_completion": False,
                    "context_tokens": 0, "wall_sec": 0.0,
                    "error": f"scc-artifact-generation-failed: {str(exc)[:200]}",
                    "error_type": "infrastructure"}
    elif variant in EXTERNAL_VARIANTS:
        artifact, ctx_tokens, err = external_artifact(
            variant, task["repo"], task["goal"], cell_dir,
            budget if budget is not None else BUDGET)
        if artifact is None:
            return {"task_success": False, "run_completion": False,
                    "context_tokens": 0, "wall_sec": 0.0,
                    "error": err, "error_type": "infrastructure"}

    started = time.monotonic()
    result = h.run_write_task(
        agent_cmd, root, task["goal"],
        validate_cmd=None, tests_cmd=None,
        artifact_path=artifact,
    )
    wall = round(time.monotonic() - started, 1)

    # The evaluator is the CANONICAL registry (never an inline snippet):
    # evaluate() raises KeyError for unknown ids, and its own four-way
    # contract is enforced by test_evaluators.py.
    try:
        task_success = _ev.evaluate(task["id"], root)
        eval_err = None
    except Exception as exc:  # evaluator crashed = infra failure, not a coding failure
        task_success, eval_err = False, f"evaluator-crash: {type(exc).__name__}: {exc}"[:300]
    return {
        "task_success": task_success,
        "run_completion": result["run_completion"],
        "context_tokens": ctx_tokens,
        "patch_produced": result["patch_produced"],
        "modified_files": len(result["modified_files"]),
        "wall_sec": wall,
        "error": eval_err or result.get("eval_output_error") or None,
        "error_type": ("evaluator-infrastructure" if eval_err else
                       ("agent-infrastructure" if not result["run_completion"] else None)),
        "evaluator_structural": task.get("structural", False),
    }


def main(argv):
    import argparse
    parser = argparse.ArgumentParser(prog="run_write_matrix.py")
    parser.add_argument("--scc-bin", default=os.environ.get("SCC_BIN") or "scc")
    parser.add_argument("--agent-cmd", default=None,
                        help="writable agent command (default: codex workspace-write)")
    parser.add_argument("--agent-label", default=None,
                        help="label for the agent (recorded in metadata; NEVER inferred — a claude command must never be recorded as codex)")
    parser.add_argument("--model-label", default=None,
                        help="label for the model, if the agent command does not encode it")
    parser.add_argument("--budget", type=int, default=None,
                        help="equal-token context budget (default: None = native-default mode; §20)")
    parser.add_argument("--out", default=str(HERE.parent / "results" / "write-matrix.json"))
    parser.add_argument("--tasks", help="comma-separated canonical task ids (default: all evaluator-backed)")
    parser.add_argument("--variants", default="raw,scc-full,aider-repomap,repomix-compress",
                        help="comma-separated variants")
    args = parser.parse_args(argv)

    tasks = SCOPED_TASKS
    if args.tasks:
        wanted = set(args.tasks.split(","))
        unknown = wanted - {t["id"] for t in SCOPED_TASKS}
        if unknown:
            print(f"unknown task ids (no evaluator): {sorted(unknown)}", file=sys.stderr)
            return 2
        tasks = [t for t in SCOPED_TASKS if t["id"] in wanted]
    variants = tuple(v.strip() for v in args.variants.split(",") if v.strip())
    agent_cmd = args.agent_cmd or WRITABLE_AGENT_CMD
    agent_label = args.agent_label or agent_cmd.split()[0]

    # Reproducibility metadata (§33).
    def _git(cwd, *a):
        try:
            return subprocess.run(["git", *a], cwd=cwd, capture_output=True,
                                  text=True, timeout=30).stdout.strip()
        except Exception:
            return "unknown"
    scc_root = HERE.parent.parent
    tasks_hash = hashlib.sha256((HERE.parent / "tasks.json").read_bytes()).hexdigest()[:16]
    evaluators_hash = hashlib.sha256((HERE / "evaluators.py").read_bytes()).hexdigest()[:16]
    meta = {
        "schema": 2,
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "agent_label": agent_label,
        "agent_cmd": agent_cmd,
        "model_label": args.model_label,
        "scc_commit": _git(scc_root, "rev-parse", "HEAD"),
        "scc_dirty": bool(_git(scc_root, "status", "--porcelain")),
        "benchmark_harness_commit": _git(HERE, "rev-parse", "HEAD") if (HERE / ".git").exists() else _git(scc_root, "rev-parse", "HEAD"),
        "tasks_corpus_hash": tasks_hash,
        "evaluators_hash": evaluators_hash,
        "mode": "writable-equal-token" if args.budget is not None else "writable-native-default",
        "requested_budget": args.budget if args.budget is not None else BUDGET,
    }

    workdir = Path(tempfile.mkdtemp(prefix="scc-write-matrix-"))
    # Disk hygiene: the workdir holds one isolated repo copy per cell
    # (GBs per full matrix). Results are fully captured in the output JSON
    # (patch text, modified files, evaluator output), so the workdir is
    # removed when the run finishes writing — including on failure, after
    # the partial results are persisted.
    results = {"meta": meta, "cells": {}}
    try:
        for variant in variants:
            for task in tasks:
                key = f"{variant}/{task['id']}"
                print(f"[matrix] {key} ...", flush=True)
                cell = run_variant(variant, task, workdir, args.scc_bin,
                                   agent_cmd=agent_cmd,
                                   agent_label=agent_label,
                                   model_label=args.model_label,
                                   budget=args.budget)
                cell["requested_budget"] = args.budget if args.budget is not None else None
                results["cells"][key] = cell
                print(f"          success={cell['task_success']} "
                      f"completion={cell['run_completion']} wall={cell['wall_sec']}s", flush=True)
    finally:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(results, indent=2))
        shutil.rmtree(workdir, ignore_errors=True)

    # Paired statistics (§31): join cells by EXACT task id; the paired set
    # for a comparison is the INTERSECTION of non-infra-error cells. A
    # cell with an infrastructure error never enters a paired array.
    ids = [t["id"] for t in tasks]
    results["summary"] = compute_summary(results["cells"], ids, list(variants))
    results["summary"]["n_tasks"] = len(ids)

    Path(args.out).write_text(json.dumps(results, indent=2))
    print(json.dumps(results["summary"], indent=2))
    return 0


def compute_summary(cells, ids, variants):
    """Paired statistics (§31) over a cells dict. Shared by main() and
    the cell-merge tooling so refills recompute identical summaries."""
    def paired_ids(*variant_names):
        ok = set(ids)
        for v in variant_names:
            for i in ids:
                cell = cells.get(f"{v}/{i}")
                if cell is None or cell.get("error"):
                    ok.discard(i)
        return sorted(ok)

    def series(v, paired):
        vals = [1.0 if cells[f"{v}/{i}"]["task_success"] else 0.0 for i in paired]
        return (sum(vals) / len(vals)) if vals else None, len(vals)

    summary = {}
    for v in variants:
        rate, n = series(v, paired_ids(v))
        summary[f"{v}_task_success"] = rate
        summary[f"{v}_n"] = n
    summary["skipped_cells"] = {
        v: {i: cells.get(f"{v}/{i}", {}).get("error")
            for i in ids if cells.get(f"{v}/{i}", {}).get("error")}
        for v in variants}

    for other in variants:
        if other == "raw":
            continue
        pair = paired_ids("raw", other)
        if not pair:
            summary[f"paired_{other}_minus_raw"] = None
            continue
        a = [1.0 if cells[f"{other}/{i}"]["task_success"] else 0.0 for i in pair]
        b = [1.0 if cells[f"raw/{i}"]["task_success"] else 0.0 for i in pair]
        mean_diff, lo, hi = h.paired_bootstrap_ci(a, b)
        summary[f"paired_{other}_minus_raw"] = {
            "n_paired": len(pair),
            "mean_diff": mean_diff,
            "ci95": [lo, hi],
            "ci_note": ("CI crosses zero — no superiority claim" if lo <= 0 <= hi
                        else "CI excludes zero"),
        }
    return summary


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
