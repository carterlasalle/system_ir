#!/usr/bin/env python3
"""Merge refilled cells from scratch matrix files into a base matrix file.

Usage:
  merge_cells.py BASE.json SCRATCH1.json [SCRATCH2.json ...] [--dry-run]

Cells from scratch files overwrite same-key cells in the base. The summary
is recomputed with the shared compute_summary() so refills yield
identical statistics to a fresh full run. Validity blocks are preserved
from the base file (a human re-classifies after inspecting the merge).
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_write_matrix import compute_summary


def main(argv):
    dry = "--dry-run" in argv
    files = [a for a in argv if not a.startswith("--")]
    base_path = Path(files[0])
    base = json.loads(base_path.read_text())
    n_over = 0
    for sp in files[1:]:
        scratch = json.loads(Path(sp).read_text())
        for key, cell in scratch["cells"].items():
            if key in base["cells"]:
                n_over += 1
            base["cells"][key] = cell
    ids = sorted({k.split("/", 1)[1] for k in base["cells"]})
    variants = sorted({k.split("/", 1)[0] for k in base["cells"]})
    base["summary"] = compute_summary(base["cells"], ids, variants)
    base["summary"]["n_tasks"] = len(ids)
    if dry:
        print(json.dumps(base["summary"], indent=1))
        print(f"would overwrite {n_over} cells in {base_path}")
        return 0
    base_path.write_text(json.dumps(base, indent=2))
    print(f"merged: overwrote {n_over} cells in {base_path}")
    print(json.dumps(base["summary"], indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
