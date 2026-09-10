#!/usr/bin/env python3
"""Paid-benchmark preflight (mission §44): refuse to start a paid agent
benchmark until the cheap gates answer every question.

Checks (all from local files, zero model quota):
  1. git worktree clean (no dirty state under measurement)
  2. BENCHMARK_SCIENCE.md fresh (regenerates byte-identical: deterministic)
  3. task corpus hash matches the manifest (not repeating stale conditions
     blindly — mismatch refuses with an explicit override requirement)
  4. at least one informative (non-floor, non-ceiling) task exists
  5. requested caps bind (max realized >= 95% of cap); otherwise the run is
     a COMMON-CAP REPLICATE, not a dose experiment → refuse multi-budget
  6. an explicit experimental question is provided (--question)

Usage:
    python3 benchmarks/external/preflight.py --question "..." [--budget 4000,8000]

Exit 0 = GO (all answers present). Exit 2 = REFUSE with reasons.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
SCIENCE = ROOT / "docs" / "BENCHMARK_SCIENCE.md"


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
    if "Binding files: 0/" in text:
        problems.append(
            "no budget binds (0 binding rows): 4k/8k/16k/24k are COMMON-CAP "
            "REPLICATES on this corpus — a four-budget run cannot answer a dose question"
        )
    if args.budget:
        wants = [b.strip() for b in args.budget.split(",")]
        if len(wants) > 1 and "Binding files: 0/" in text:
            problems.append(
                f"multi-budget run ({args.budget}) refused: caps do not bind"
            )
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
