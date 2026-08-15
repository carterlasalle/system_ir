#!/usr/bin/env python3
"""Aider RepoMap adapter (Wave 15 external benchmark suite).

Invokes the PINNED aider repo-map (benchmarks/external-lock.json:
Aider-AI/aider @ 5dc9490bb) through its real RepoMap API and writes the map
for a repository under an equal-token budget.

Usage:
    python3 aider_adapter.py <repo> <token_budget> <out_dir>

Writes:
    <out_dir>/aider-map.txt

Prints JSON on stdout:
    {"ok": true, "tool": "aider", "tokens": <int>, "files": <int>,
     "artifact": "<path>"}

Exit codes:
    0  success
    2  SKIPPED-UNINSTALLED — aider (or its python deps) is not installed;
       prints {"ok": false, "error": "SKIPPED-UNINSTALLED: ..."} and the
       harness reports the variant as SKIPPED-UNINSTALLED.
    1  other failure

Installation (pinned):
    pip install "git+https://github.com/Aider-AI/aider.git@5dc9490bb35f9729ef2c95d00a19ccd30c26339c"
"""

import json
import os
import shutil
import sys
from pathlib import Path

LOCKED_AIDER_COMMIT = "5dc9490bb35f9729ef2c95d00a19ccd30c26339c"


def src_files(directory):
    """Absolute paths of the repo's source files (mirrors aider's own
    find_src_files; skips state/vendored dirs)."""
    skip_dirs = {".git", ".scc", "node_modules", "__pycache__", ".venv", "venv", "target", ".aider.tags.cache.v3", ".aider.tags.cache.v4"}
    out = []
    for root, dirs, files in os.walk(directory):
        dirs[:] = [d for d in dirs if d not in skip_dirs]
        for f in files:
            out.append(os.path.join(root, f))
    return out


def estimate_tokens(text):
    # Deterministic chars/4 heuristic — same rule the harness and the scc
    # side use, so adapter-reported tokens are comparable across variants.
    if not text:
        return 0
    return max(1, len(text) // 4)


def build_repomap(repo_abs, budget):
    """Construct the pinned aider RepoMap for the budget. Returns the map
    text, or raises AiderMissing when aider is unavailable."""
    if not shutil.which("aider"):
        raise AiderMissing("aider CLI not found on PATH")

    try:
        from aider.io import InputOutput  # noqa: F401
        from aider.models import Model
        from aider.repomap import RepoMap
    except ImportError as exc:
        raise AiderMissing(f"aider python package not importable: {exc}") from exc

    try:
        model = Model("gpt-4o")
    except Exception:
        model = None  # token counting falls back to the chars/4 heuristic

    io = InputOutput(pretty=False, yes=True)
    try:
        rm = RepoMap(
            map_tokens=budget,
            root=repo_abs,
            main_model=model,
            io=io,
            refresh="manual",
        )
    except TypeError:
        # Older pinned signatures: (map_tokens, root, main_model, io)
        rm = RepoMap(budget, repo_abs, model, io)

    other_files = src_files(repo_abs)
    # chat_files=[] -> whole-repo map at max_map_tokens (equal-token mode).
    # The map can land within +/-15% of the budget (aider's binary search);
    # the adapter reports the actual estimate.
    return rm.get_repo_map([], other_files) or ""


class AiderMissing(Exception):
    pass


def main(argv):
    if len(argv) != 4:
        print(
            json.dumps(
                {
                    "ok": False,
                    "error": (
                        f"usage: {argv[0]} <repo> <token_budget> <out_dir> "
                        f"(pinned aider commit {LOCKED_AIDER_COMMIT})"
                    ),
                }
            )
        )
        return 1

    repo, budget_s, out_dir = argv[1], argv[2], argv[3]
    try:
        budget = int(budget_s)
    except ValueError:
        print(json.dumps({"ok": False, "error": f"invalid token budget: {budget_s}"}))
        return 1
    if budget <= 0:
        print(json.dumps({"ok": False, "error": "token budget must be positive"}))
        return 1

    repo_abs = os.path.abspath(repo)
    if not os.path.isdir(repo_abs):
        print(json.dumps({"ok": False, "error": f"not a directory: {repo}"}))
        return 1

    try:
        os.makedirs(out_dir, exist_ok=True)
        # Suppress aider's progress UI (tqdm/Spinner) on stdout so the JSON
        # contract stays clean; generation errors still hit stderr.
        import contextlib
        with open(os.devnull, "w") as devnull, contextlib.redirect_stdout(devnull):
            map_text = build_repomap(repo_abs, budget)
    except AiderMissing as exc:
        print(json.dumps({"ok": False, "error": f"SKIPPED-UNINSTALLED: {exc}"}))
        return 2
    except Exception as exc:  # aider internals can throw on odd repos
        print(json.dumps({"ok": False, "error": f"aider repomap failed: {exc}"}))
        return 1

    artifact = os.path.join(out_dir, "aider-map.txt")
    with open(artifact, "w") as fh:
        fh.write(map_text)

    print(
        json.dumps(
            {
                "ok": True,
                "tool": "aider",
                "tokens": estimate_tokens(map_text),
                "files": len(src_files(repo_abs)),
                "artifact": artifact,
            }
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
