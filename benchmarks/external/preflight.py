#!/usr/bin/env python3
"""Paid-benchmark preflight (mission §44): refuse to start a paid agent
benchmark until the cheap gates answer every question.

Checks (all local, zero model quota):
  1. git worktree clean (no dirty state under measurement)
  2. BENCHMARK_SCIENCE.md fresh (regenerates byte-identical: deterministic)
  3. current benchmarks/tasks.json sha256[:16] matches a VALID manifest
     corpus (comparisons against a stale corpus refuse outright)
  4. at least one informative (non-floor, non-ceiling) task exists
  5. PROPOSED caps bind: for each --budget value, a local
     run_write_matrix.py --dry-run builds the actual treatment artifacts
     and the max realized context must reach >= 95% of the cap.
     Historical binding is NOT consulted (a past non-binding corpus says
     nothing about a future proposal). Multi-budget dose runs refuse when
     no proposed cap binds; a single non-binding budget warns but passes
     (absolute-performance questions need no scarcity).
  6. an explicit experimental question is provided (--question)

Usage:
    python3 benchmarks/external/preflight.py --question "..." [--budget 4000,8000]

Exit 0 = GO (all answers present). Exit 2 = REFUSE with reasons.
"""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
SCIENCE = ROOT / "docs" / "BENCHMARK_SCIENCE.md"
RESULTS = HERE.parent / "results"
TASKS = HERE.parent / "tasks.json"
BINDING_RATIO = 0.95


def sh(*args: str) -> str:
    return subprocess.run(
        list(args), capture_output=True, text=True, cwd=ROOT
    ).stdout.strip()


def main() -> int:
    import argparse

    ap = argparse.ArgumentParser()
    ap.add_argument("--question", default="")
    ap.add_argument("--budget", default="")
    args = ap.parse_args()

    problems: list[str] = []

    if sh("git", "status", "--porcelain"):
        problems.append("worktree dirty: commit or stash before measuring")

    # determinism = freshness: regen must be byte-identical
    before = SCIENCE.read_bytes()
    r = subprocess.run(
        [sys.executable, str(HERE / "bench_science.py")],
        capture_output=True, cwd=ROOT,
    )
    if r.returncode != 0 or SCIENCE.read_bytes() != before:
        problems.append("BENCHMARK_SCIENCE.md not reproducible from checked-in results")

    text = SCIENCE.read_text()
    if "CURRENT BLIND PERFORMANCE = VERIFIED" not in text:
        problems.append("blind status not verified (run aggregate-only bench atlas --blind)")

    # Current corpus hash vs the manifest's VALID corpora (never assume
    # task definitions stayed equivalent across corpora).
    current_hash = hashlib.sha256(TASKS.read_bytes()).hexdigest()[:16]
    valid_corpora: set[str] = set()
    for fp in sorted(RESULTS.glob("write-matrix-*.json")):
        if fp.name.endswith(".invalid.json"):
            continue
        try:
            d = json.loads(fp.read_text())
        except Exception:
            continue
        if "cells" not in d or "meta" not in d:
            continue
        if d.get("validity", {}).get("valid", True) is False:
            continue
        h = d["meta"].get("tasks_corpus_hash")
        if h:
            valid_corpora.add(h)
    if current_hash not in valid_corpora:
        problems.append(
            f"current tasks.json corpus {current_hash} matches no VALID manifest "
            f"corpus ({sorted(valid_corpora) or 'none'}): comparisons would pool "
            "across task definitions — record a new baseline first"
        )
    else:
        print(f"corpus: current tasks.json {current_hash} matches a valid baseline")

    # Proposed-cap binding probe: dry-build THIS proposal's artifacts.
    binding: dict[int, tuple[int, float]] = {}
    if args.budget:
        wants = [int(b.strip()) for b in args.budget.split(",") if b.strip()]
        for cap in wants:
            with tempfile.TemporaryDirectory(prefix="preflight-probe-") as td:
                out = str(Path(td) / "probe.json")
                r = subprocess.run(
                    [sys.executable, str(HERE / "run_write_matrix.py"),
                     "--dry-run", "--budget", str(cap), "--out", out,
                     "--question", args.question or "preflight cap-binding probe"],
                    capture_output=True, text=True, cwd=ROOT,
                )
                if r.returncode != 0:
                    problems.append(f"cap probe failed at budget {cap}: {r.stderr.strip()[-300:]}")
                    continue
                try:
                    pd = json.loads(Path(out).read_text())
                except Exception as exc:
                    problems.append(f"cap probe unreadable at budget {cap}: {exc}")
                    continue
                mx = 0
                for c in pd.get("cells", {}).values():
                    if isinstance(c, dict) and isinstance(c.get("context_tokens"), (int, float)):
                        mx = max(mx, int(c["context_tokens"]))
                binding[cap] = (mx, mx / cap if cap else 0.0)
                state = "BINDING" if binding[cap][1] >= BINDING_RATIO else "non-binding"
                print(f"cap probe: budget {cap}: max realized {mx} (util {binding[cap][1]:.3f}) → {state}")
        if len(wants) > 1 and binding and all(u < BINDING_RATIO for _, u in binding.values()):
            problems.append(
                f"multi-budget dose run ({args.budget}) refused: none of the PROPOSED "
                "caps binds (dry-run max realized < 95% of every cap) — this would be "
                "a COMMON-CAP REPLICATE, not a dose experiment"
            )
        for cap, (mx, u) in binding.items():
            if u < BINDING_RATIO:
                print(f"warning: budget {cap} does not bind this corpus (max realized {mx}); "
                      "absolute-performance questions are unaffected")
    if "floor" not in text or "informative" not in text:
        problems.append("no task floor/ceiling diagnostics available")
    if not args.question.strip():
        problems.append("no experimental question (--question \"...\" required)")

    if problems:
        print("PREFLIGHT REFUSE:")
        for p in problems:
            print(f"  - {p}")
        return 2
    print("PREFLIGHT GO: cheap gates answer every question.")
    print("  (paid model benchmark calls made by this check: 0)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
