#!/usr/bin/env python3
"""Repomix adapter (Wave 15 external benchmark suite).

Invokes the PINNED repomix (benchmarks/external-lock.json:
yamadashy/repomix @ e3b15a406) to pack a repository, then applies
equal-token mode: complete files are kept in repomix's own ordering until
the budget is reached (a file is never truncated mid-file).

Usage:
    python3 repomix_adapter.py <repo> <token_budget> <out_dir> [--compress]

Writes:
    <out_dir>/repomix.txt   (the budget-capped, complete-file pack)
    <out_dir>/repomix-full.xml  (repomix's full output, for debugging)

Prints JSON on stdout:
    {"ok": true, "tool": "repomix", "tokens": <int>, "files": <int>,
     "artifact": "<path>", "compressed": <bool>}

Exit codes:
    0  success
    2  SKIPPED-UNINSTALLED — repomix is not installed; prints
       {"ok": false, "error": "SKIPPED-UNINSTALLED: ..."}
    1  other failure

Installation (pinned):
    npm install -g "github:yamadashy/repomix#e3b15a406ed78d8a463620a032a059ce911bfc0e"
or rely on the npx fallback (network required on first use):
    npx --yes "github:yamadashy/repomix#e3b15a406ed78d8a463620a032a059ce911bfc0e"
"""

import html
import json
import os
import re
import shutil
import subprocess
import sys

LOCKED_REPOMIX_COMMIT = "e3b15a406ed78d8a463620a032a059ce911bfc0e"

FILE_RE = re.compile(r'<file path="([^"]+)"[^>]*>([\s\S]*?)</file>')


def estimate_tokens(text):
    if not text:
        return 0
    return max(1, len(text) // 4)


def repomix_command(repo_abs, out_file, compress):
    """argv for the pinned repomix CLI. Prefers a globally installed
    `repomix`; falls back to npx with the pinned git ref."""
    if shutil.which("repomix"):
        base = ["repomix"]
    elif shutil.which("npx"):
        base = ["npx", "--yes", f"github:yamadashy/repomix#{LOCKED_REPOMIX_COMMIT}"]
    else:
        return None
    args = [
        base[0],
        *base[1:],
        repo_abs,
        "--style", "xml",
        "-o", out_file,
        "--no-git-sort-by-changes",
        "--no-security-check",
        "--quiet",
    ]
    if compress:
        args.append("--compress")
    return args


def run_repomix(argv):
    repo_abs = os.path.abspath(argv[1])
    budget = int(argv[2])
    out_dir = os.path.abspath(argv[3])
    compress = "--compress" in argv[4:]

    if not os.path.isdir(repo_abs):
        return {"ok": False, "error": f"not a directory: {argv[1]}"}, 1
    os.makedirs(out_dir, exist_ok=True)
    full_xml = os.path.join(out_dir, "repomix-full.xml")
    argv_cmd = repomix_command(repo_abs, full_xml, compress)
    if argv_cmd is None:
        return {
            "ok": False,
            "error": "SKIPPED-UNINSTALLED: neither `repomix` nor `npx` found on PATH "
                     f"(pinned commit {LOCKED_REPOMIX_COMMIT})",
        }, 2

    proc = subprocess.run(argv_cmd, capture_output=True, text=True, timeout=600)
    if proc.returncode != 0:
        detail = proc.stderr.strip() or proc.stdout.strip()
        return {
            "ok": False,
            "error": f"repomix pack failed (exit {proc.returncode}): {detail[:500]}",
        }, 1

    with open(full_xml, "r", errors="replace") as fh:
        full = fh.read()

    # Equal-token mode: complete files in repomix's ordering, never a
    # mid-file truncation. XML content is entity-escaped; unescape before
    # counting so token estimates match the artifact the agent receives.
    sections = []
    for match in FILE_RE.finditer(full):
        path = match.group(1)
        content = html.unescape(match.group(2))
        sections.append((path, content))

    if not sections:
        # The pinned CLI's XML shape changed; fall back to the whole output
        # (a single "file"), which still satisfies never-truncate.
        sections = [("<unknown>", full)]

    kept = []
    total = 0
    for path, content in sections:
        tokens = estimate_tokens(content)
        if kept and total + tokens > budget:
            break  # budget reached — stop at a file boundary
        kept.append((path, content))
        total += tokens

    packed = "\n\n".join(f"## File: {path}\n{content}" for path, content in kept)
    artifact = os.path.join(out_dir, "repomix.txt")
    with open(artifact, "w") as fh:
        fh.write(packed)

    return {
        "ok": True,
        "tool": "repomix",
        "tokens": total,
        "files": len(kept),
        "artifact": artifact,
        "compressed": compress,
    }, 0


def main(argv):
    if len(argv) not in (4, 5):
        print(
            json.dumps(
                {
                    "ok": False,
                    "error": (
                        f"usage: {argv[0]} <repo> <token_budget> <out_dir> [--compress] "
                        f"(pinned repomix commit {LOCKED_REPOMIX_COMMIT})"
                    ),
                }
            )
        )
        return 1
    try:
        int(argv[2])
    except ValueError:
        print(json.dumps({"ok": False, "error": f"invalid token budget: {argv[2]}"}))
        return 1

    payload, code = run_repomix(argv)
    print(json.dumps(payload))
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv))
