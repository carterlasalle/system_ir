#!/usr/bin/env python3
"""Final statistical report (mission §50) over the valid writable matrices.

Reads benchmarks/results/write-matrix-*.json files, honors each file's
`validity.valid` flag (§34: invalid files never enter claims), and prints
per-budget micro/macro success rates and PAIRED bootstrap CIs joined by
exact task id (§31). No superiority claim is made when a CI crosses zero.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

R = Path(__file__).resolve().parent.parent / "results"
VARIANTS = ("raw", "scc-full", "aider-repomap", "repomix-compress")


def load_valid(prefix: str):
    out = {}
    for p in sorted(R.glob(f"write-matrix-{prefix}*.json")):
        d = json.loads(p.read_text())
        if not d.get("validity", {}).get("valid", True):
            continue
        if "summary" not in d:
            continue
        out[p.name] = d
    return out


def micro(cells: dict, variant: str, ids: list[str]) -> float | None:
    vals = [cells.get(f"{variant}/{i}", {}).get("task_success") for i in ids]
    vals = [v for v in vals if v is not None]
    return (sum(vals) / len(vals)) if vals else None


def repo_macro(cells: dict, variant: str, ids_by_repo: dict[str, list[str]]) -> float | None:
    rates = []
    for repo, ids in ids_by_repo.items():
        m = micro(cells, variant, ids)
        if m is not None:
            rates.append(m)
    return (sum(rates) / len(rates)) if rates else None


def report(label: str, d: dict):
    cells = d["cells"]
    ids = sorted({k.split("/", 1)[1] for k in cells})
    repos: dict[str, list[str]] = {}
    for i in ids:
        repo = d["cells"].get(f"raw/{i}", {}).get("repo") or i.split(".")[0]
        repos.setdefault(repo, []).append(i)

    print(f"\n== {label} ==")
    print(f"   agent={d['meta'].get('agent_label')} model={d['meta'].get('model_label')} "
          f"mode={d['meta'].get('mode')} budget={d['meta'].get('requested_budget')}")
    for v in VARIANTS:
        m = micro(cells, v, ids)
        mac = repo_macro(cells, v, repos)
        ms = f"{m:.1%}" if m is not None else "n/a"
        macs = f"{mac:.1%}" if mac is not None else "n/a"
        print(f"   {v:<18} micro={ms:>6}  macro(repo)={macs:>6}")
    for other in VARIANTS[1:]:
        p = d["summary"].get(f"paired_{other}_minus_raw")
        if p:
            verdict = ("CROSSES ZERO — no superiority claim"
                       if p["ci95"][0] <= 0 <= p["ci95"][1] else "EXCLUDES ZERO")
            print(f"   paired {other} - raw: +{p['mean_diff']:.3f} "
                  f"CI95=[{p['ci95'][0]:.3f}, {p['ci95'][1]:.3f}] n={p['n_paired']}  {verdict}")


def main() -> int:
    seen = False
    for name, d in load_valid("4k").items():
        report(name, d)
        seen = True
    for name, d in load_valid("8k").items():
        report(name, d)
        seen = True
    for name, d in load_valid("16k").items():
        report(name, d)
        seen = True
    for name, d in load_valid("24k").items():
        report(name, d)
        seen = True
    for name, d in load_valid("native-default").items():
        report(name, d)
        seen = True
    if not seen:
        print("no valid matrices found")
        return 1
    print("\n(invalid historical matrices are excluded from all claims; see each file's validity block)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
