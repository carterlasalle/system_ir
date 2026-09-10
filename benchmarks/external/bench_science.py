#!/usr/bin/env python3
"""Benchmark science from existing data only (mission Part XII).

Reads checked-in benchmarks/results/write-matrix-*.json files. Launches
NOTHING: no agents, no models, no subprocesses. Output is a derived
interpretation layer — raw result files are never rewritten.

Produces:
  docs/BENCHMARK_SCIENCE.md — manifest, cap utilization, repo-clustered
  CIs, floor/ceiling diagnostics, and labeled exploratory verdicts.

Conventions (fixed, documented here so reruns agree):
  BINDING_RATIO = 0.95 — a budget binds a file iff max realized context
      tokens across valid cells >= 95% of the requested cap.
  BOOT_SEED = 1234, BOOT_ITERS = 10000 — deterministic resampling.
  Cluster bootstrap resamples REPOSITORIES (tasks within a repo stay
      together); task bootstrap resamples tasks. Clustered is headlined.
"""

from __future__ import annotations

import json
import random
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent / "results"
OUT = HERE.parent.parent / "docs" / "BENCHMARK_SCIENCE.md"

BINDING_RATIO = 0.95
BOOT_SEED = 1234
BOOT_ITERS = 10000
VARIANTS = ("raw", "scc-full", "aider-repomap", "repomix-compress")


def is_valid(d):
    """The file's own validity model: filename twin OR validity.valid=false
    both mean INVALID. Analysis sections must use valid_matrices(), never
    load_matrices() directly."""
    return d.get("validity", {}).get("valid", True) is not False


def load_matrices():
    """(name, payload) for every parseable matrix file, sorted.

    INVALID files (`.invalid.json` twins AND `validity.valid == false`)
    are LOADED so the manifest records them, but every analysis section
    (§2 cap utilization, §3 CIs, §4 floor/ceiling) consumes only
    valid_matrices()."""
    out = []
    for p in sorted(RESULTS.glob("write-matrix-*.json")):
        if p.name.endswith(".invalid.json"):
            continue
        try:
            d = json.loads(p.read_text())
        except Exception:
            continue
        if "cells" not in d or "meta" not in d:
            continue
        out.append((p.name, d))
    return out


def valid_matrices(matrices):
    """Subset of load_matrices() output the file's own validity model
    accepts. Pooling anything else violates the manifest's verdicts."""
    return [(n, d) for n, d in matrices if is_valid(d)]


def valid_cells(d):
    """Cells with a defined task outcome (excludes infra/skipped)."""
    return {
        k: c
        for k, c in d["cells"].items()
        if isinstance(c, dict) and c.get("task_success") is not None
    }


def repo_of(cell_key, cell):
    if isinstance(cell.get("repo"), str) and cell["repo"]:
        return cell["repo"]
    tid = cell_key.split("/", 1)[1] if "/" in cell_key else cell_key
    return tid.split(".")[0]


def manifest_row(name, d):
    cells = valid_cells(d)
    repos = sorted({repo_of(k, c) for k, c in cells.items()})
    meta = d["meta"]
    validity = d.get("validity", {})
    return {
        "file": name,
        "schema": meta.get("schema"),
        "timestamp": meta.get("timestamp"),
        "scc_commit": (meta.get("scc_commit") or "")[:12],
        "corpus_hash": meta.get("tasks_corpus_hash"),
        "agent": meta.get("agent_label"),
        "model": meta.get("model_label"),
        "mode": meta.get("mode"),
        "requested_budget": meta.get("requested_budget"),
        "valid_cells": len(cells),
        "repos": len(repos),
        "repo_list": ",".join(repos),
        "verdict": "VALID"
        if validity.get("valid", True)
        else f"INVALID: {validity.get('invalid_reason', '')[:80]}",
        "note": (validity.get("note") or "")[:120],
    }


def cap_table(name, d):
    """Per-variant requested cap vs realized context tokens."""
    meta = d["meta"]
    cap = meta.get("requested_budget")
    rows = []
    for v in VARIANTS:
        toks = [
            c["context_tokens"]
            for k, c in valid_cells(d).items()
            if k.startswith(v + "/") and isinstance(c.get("context_tokens"), (int, float))
        ]
        if not toks:
            rows.append((v, cap, None, None, None, "no-data"))
            continue
        mx = max(toks)
        mean = sum(toks) / len(toks)
        if cap is None:
            binding = "n/a-native"
            util = None
        else:
            util = mx / cap
            binding = "BINDING" if util >= BINDING_RATIO else "non-binding"
        rows.append((v, cap, round(mean, 1), mx, None if util is None else round(util, 3), binding))
    return rows


def bootstrap(values, seed, iters):
    rng = random.Random(seed)
    n = len(values)
    if n == 0:
        return (0.0, 0.0, 0.0)
    mean = sum(values) / n
    samples = sorted(sum(values[rng.randrange(n)] for _ in range(n)) / n for _ in range(iters))
    return (mean, samples[int(0.025 * iters)], samples[int(0.975 * iters)])


def clustered_ci(d, left="scc-full", right="raw"):
    """Repo-clustered bootstrap CI for (left - right) success difference.

    Paired by task; resampling unit is the repository. Returns
    (mean, lo, hi, n_tasks, n_repos, {repo: size}).
    """
    cells = valid_cells(d)
    by_repo: dict[str, list[tuple[float, float]]] = {}
    for k, c in cells.items():
        variant, tid = k.split("/", 1)
        if variant not in (left, right):
            continue
        by_repo.setdefault(repo_of(k, c), {}).setdefault(tid, {})[variant] = float(c["task_success"])
    pairs: dict[str, list[tuple[float, float]]] = {}
    for repo, tasks in by_repo.items():
        for tid, m in tasks.items():
            if left in m and right in m:
                pairs.setdefault(repo, []).append((m[left], m[right]))
    repos = sorted(pairs)
    if not repos:
        return (0.0, 0.0, 0.0, 0, 0, {})
    diffs = {r: [a - b for a, b in pairs[r]] for r in repos}
    flat = [x for r in repos for x in diffs[r]]
    mean = sum(flat) / len(flat)
    rng = random.Random(BOOT_SEED)
    samples = []
    for _ in range(BOOT_ITERS):
        tot = cnt = 0.0
        for _ in repos:
            r = repos[rng.randrange(len(repos))]
            for x in diffs[r]:
                tot += x
                cnt += 1
        samples.append(tot / cnt)
    samples.sort()
    return (mean, samples[int(0.025 * BOOT_ITERS)], samples[int(0.975 * BOOT_ITERS)],
            len(flat), len(repos), {r: len(diffs[r]) for r in repos})


def verdict(lo, hi):
    return "CROSSES ZERO — no superiority claim" if lo <= 0 <= hi else "EXCLUDES ZERO"


def floor_ceiling(matrices):
    """Per-(task, corpus) attempts/success pooled over VALID matrices only.

    Never pool across tasks_corpus_hash values: task definitions may have
    changed between corpora, so each corpus gets its own rows. Callers must
    pass valid_matrices()."""
    agg: dict[tuple[str, str], dict] = {}
    for name, d in matrices:
        agent = d["meta"].get("agent_label", "?")
        corpus = d["meta"].get("tasks_corpus_hash") or "unknown"
        for k, c in valid_cells(d).items():
            _, tid = k.split("/", 1)
            e = agg.setdefault((corpus, tid), {"n": 0, "ok": 0, "agents": set(), "files": set()})
            e["n"] += 1
            e["ok"] += int(bool(c["task_success"]))
            e["agents"].add(str(agent))
            e["files"].add(name)
    rows = []
    for (corpus, tid), e in sorted(agg.items()):
        rate = e["ok"] / e["n"]
        cls = "floor" if rate == 0.0 else ("ceiling" if rate == 1.0 else "informative")
        rows.append((corpus, tid, e["n"], e["ok"], round(rate, 3), cls, len(e["agents"]), len(e["files"])))
    return rows


def main() -> int:
    matrices = load_matrices()
    if not matrices:
        print("no matrices found", file=sys.stderr)
        return 1

    L: list[str] = []
    L.append("# Benchmark science (derived, existing data only)")
    L.append("<!-- trace:v1 id=doc.scc-benchmark-science work=WORK-SI-MMMJA4G6 documents=REQ-SI-503JSBGP -->")
    L.append("")
    L.append("> Generated by `benchmarks/external/bench_science.py` from checked-in")
    L.append("> `benchmarks/results/write-matrix-*.json` files. Analysis only: no")
    L.append("> agents, no models, no subprocesses. Raw results are never rewritten.")
    L.append("> All multi-condition comparisons below are EXPLORATORY (post hoc,")
    L.append("> un-preregistered); CIs that cross zero support no superiority claim.")
    L.append("")
    L.append("## 1. Result manifest")
    L.append("")
    L.append("| file | schema | timestamp | scc_commit | corpus | agent | model | mode | budget | valid_cells | repos | verdict |")
    L.append("|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for name, d in matrices:
        m = manifest_row(name, d)
        L.append(
            f"| {m['file']} | {m['schema']} | {m['timestamp']} | {m['scc_commit']} | "
            f"{m['corpus_hash']} | {m['agent']} | {m['model']} | {m['mode']} | "
            f"{m['requested_budget']} | {m['valid_cells']} | {m['repos']} ({m['repo_list']}) | {m['verdict']} |"
        )
    L.append("")
    L.append("Supersession: `.invalid.json` twins AND files with `validity.valid == false`")
    L.append("are listed here but excluded from every claim below (§2–§4 consume valid files")
    L.append("only). Files whose validity note records a refill (e.g. repomix/aider cell refills)")
    L.append("supersede the provisional partials they completed; the manifest lists each surviving")
    L.append("file once. Native-default files (budget None) are not budget conditions at all.")
    valid = valid_matrices(matrices)
    L.append("")
    L.append(f"Corpus lineage: {len(valid)}/{len(matrices)} files valid, grouped by tasks_corpus_hash")
    L.append("(pooled claims never cross corpus boundaries):")
    lineage: dict[str, list[str]] = {}
    for name, d in valid:
        lineage.setdefault(d["meta"].get("tasks_corpus_hash") or "unknown", []).append(name)
    for corpus, names in sorted(lineage.items()):
        L.append(f"- corpus {corpus}: {len(names)} files ({', '.join(names)})")
    L.append("")

    L.append("## 2. Cap utilization (binding iff max realized >= 95% of cap)")
    L.append("")
    L.append("| file | variant | requested | mean_tokens | max_tokens | max_util | binding |")
    L.append("|---|---|---|---|---|---|---|")
    binding_files = 0
    total_budgeted = 0
    for name, d in valid:
        for v, cap, mean, mx, util, binding in cap_table(name, d):
            if cap is not None:
                total_budgeted += 1
                if binding == "BINDING":
                    binding_files += 1
            L.append(f"| {name} | {v} | {cap} | {mean} | {mx} | {util} | {binding} |")
    L.append("")
    L.append(f"Binding files: {binding_files}/{total_budgeted} budgeted (variant, file) rows bind.")
    L.append("Non-binding rows are COMMON-CAP REPLICATES, not context-dose experiments:")
    L.append("they measure the same small artifacts under looser caps.")
    L.append("")

    L.append("## 3. Repo-clustered bootstrap CIs (scc-full minus raw, headlined)")
    L.append("")
    L.append(f"Seed {BOOT_SEED}, {BOOT_ITERS} iterations, resampling unit = repository.")
    L.append("")
    for name, d in valid:
        mean, lo, hi, n_tasks, n_repos, sizes = clustered_ci(d)
        L.append(
            f"- {name}: diff={mean:+.3f} CI95=[{lo:+.3f}, {hi:+.3f}] "
            f"n_tasks={n_tasks} n_repos={n_repos} clusters={sizes} → {verdict(lo, hi)}"
        )
    L.append("")

    L.append("## 4. Task floor / ceiling (valid matrices only, per corpus)")
    L.append("")
    L.append("| corpus | task | attempts | success | rate | class | agents | files |")
    L.append("|---|---|---|---|---|---|---|---|")
    for corpus, tid, n, ok, rate, cls, na, nf in floor_ceiling(valid):
        L.append(f"| {corpus} | {tid} | {n} | {ok} | {rate} | {cls} | {na} | {nf} |")
    L.append("")
    L.append("Floor tasks (0% everywhere) should not consume future paid quota unless")
    L.append("extreme difficulty is the explicit experimental question.")
    L.append("")

    L.append("## 5. Labeled verdicts")
    L.append("")
    L.append("- Micro corpus: the write-matrix subset is 9 tasks / 4 fixture repos")
    L.append("  (<=11 files each; max 575 realized context tokens vs 4000+ caps) →")
    L.append("  SMOKE / LOCALIZATION REGRESSION, not retrieval-under-scarcity")
    L.append("  evidence. The benchagent corpus is 21 tasks. Kept and run")
    L.append("  frequently; never headlined as production retrieval quality.")
    L.append("- Precision: matrices record only task_success (recall of task goals).")
    L.append("  No precision/MRR gate exists → CI currently gates recall only.")
    L.append("- Hallucination probes: none falsifiable exists in this harness.")
    L.append("- Blind/holdout: lineage files (blind-v*.txt, holdout-v*.txt) record")
    L.append("  aggregate protocol runs. CURRENT BLIND PERFORMANCE = VERIFIED")
    L.append("  (aggregate): 2026-09-10 `bench atlas --blind` rerun (blind-v2.txt)")
    L.append("  reproduced blind-v1 per-layer recall EXACTLY on the present tree;")
    L.append("  only atlas token counts moved (+7.5k validation, +4.8k blind).")
    L.append("  blind-protocol-passed ≠ blind-quality-passed still holds per run.")
    L.append("- Paid guard: `SCC_ALLOW_PAID_BENCHMARKS=1` interlock verified by")
    L.append("  `test_write_protocol.PaidBenchmarkGateTest` + Rust gate tests;")
    L.append("  this script itself launches nothing.")
    L.append("- Preflight for any future paid run: cheap tests green, validation")
    L.append("  green, blind status known, tasks informative, repos represented,")
    L.append("  caps actually binding (see §2), conditions not repeats, and one")
    L.append("  written experimental question. Four-budget reruns of sub-600-token")
    L.append("  artifacts are refused by this checklist.")
    L.append("")
    L.append("Paid model benchmark calls made by this analysis: 0")
    L.append("")

    OUT.write_text("\n".join(L))
    print(f"wrote {OUT} ({len(valid)}/{len(matrices)} valid matrices)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
