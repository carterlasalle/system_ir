#!/usr/bin/env python3
"""compare_waves.py — per-layer deltas + generalization efficiency between
two `scc bench atlas` result files (holdout-v3.txt / blind-v1.txt format).

Usage:
    python3 benchmarks/compare_waves.py OLD_RESULTS NEW_RESULTS

Both files contain the per-layer aggregates table produced by
`scc bench atlas --holdout` (columns: development, validation, gap) or
`scc bench atlas --blind` (columns: validation, blind, gap). The script
prints, for every layer present in both files, the delta of each shared
numeric column (new - old), and the generalization efficiency for the whole
wave:

    efficiency = validation_delta / development_delta

computed on the `overall (gate)` row. efficiency > 1 means the validation
corpus improved MORE than the dev corpus did across the wave (gains
generalize); 0 < efficiency < 1 means part of the dev gain generalized;
efficiency <= 0 or a zero development delta means the wave's dev gains did
not transfer to unseen repos. A positive efficiency with both deltas
negative (both corpora regressed) is reported as a regression, not a
transfer.

The per-layer table parser reads the fixed line format the Rust emitter
writes: a header line whose first token is `layer`, then one row per layer
with the layer name first and the numeric columns after it.
"""

import re
import sys
from collections import OrderedDict

LAYER_ROW = re.compile(r"^([a-z_ ()]+)\s+([-+0-9.]+)\s+([-+0-9.]+)\s+([-+0-9.]+)$")


def parse_results(path: str) -> OrderedDict:
    """Return {layer: {column: value}} for the per-layer aggregates table."""
    rows: OrderedDict = OrderedDict()
    header = None
    with open(path, encoding="utf-8") as fh:
        for raw in fh:
            line = raw.rstrip("\n")
            if header is None:
                if line.startswith("layer"):
                    header = line.split()
                continue
            if not line.strip():
                continue  # table ends at the blank line after the rows
            m = LAYER_ROW.match(line)
            if not m:
                break  # non-row line (scored:/precision:/...) ends the table
            layer = m.group(1).strip()
            values = [float(m.group(2)), float(m.group(3)), float(m.group(4))]
            row = {}
            for col, val in zip(header[1:], values):
                row[col] = val
            rows[layer] = row
    if not rows:
        raise SystemExit(f"error: no per-layer table found in {path}")
    return rows


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    old_path, new_path = sys.argv[1], sys.argv[2]
    old = parse_results(old_path)
    new = parse_results(new_path)

    layers = [l for l in old if l in new]
    if not layers:
        raise SystemExit("error: no layers shared between the two result files")
    cols = [c for c in next(iter(old.values())) if c in next(iter(new.values()))]

    print(f"compare_waves: {old_path} -> {new_path}")
    print(f"{'layer':<18} " + "".join(f"{c:>14}" for c in cols) + f"{'efficiency':>14}")
    for layer in layers:
        deltas = {c: new[layer][c] - old[layer][c] for c in cols}
        eff = ""
        if layer == "overall (gate)":
            eff = efficiency(deltas)
        cells = "".join(f"{deltas[c]:>+14.3f}" for c in cols)
        print(f"{layer:<18} {cells}{eff:>14}")


def efficiency(deltas: dict) -> str:
    dev = deltas.get("development")
    val = deltas.get("validation")
    if dev is None or val is None:
        return "n/a"
    if abs(dev) < 1e-9:
        return "n/a (dev delta 0)"
    ratio = val / dev
    if val < 0 and dev < 0:
        return f"{ratio:+.2f} (regression)"
    return f"{ratio:+.2f}"


if __name__ == "__main__":
    main()
