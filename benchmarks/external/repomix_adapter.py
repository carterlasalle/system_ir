#!/usr/bin/env python3
"""Repomix adapter (Wave 15 external benchmark suite).

Invokes the PINNED repomix (benchmarks/external-lock.json:
yamadashy/repomix @ e3b15a406) to pack a repository, then applies
equal-token mode: complete files are kept in repomix's own ordering until
the budget is reached (a file is never truncated mid-file).

The installed repomix MUST be the pinned commit. The adapter verifies the
pin against the lock and hard-errors with `PIN-MISMATCH` (exit 3) when a
mismatch is demonstrable, or `PIN-UNVERIFIED` (exit 4) when the commit
cannot be proven — no silent floating versions and no version-only
acceptance. Verification sources, in order of strength:
  1. the installed package.json `gitHead` (npm records the resolved commit
     for git-ref installs) — must equal the locked commit;
  2. the npm global install's `resolved` path pointing at the pinned source
     checkout (~/.scc-bench/repomix, override SCC_REPOMIX_SRC_DIR) whose
     git HEAD is the locked commit AND whose declared version equals the
     installed version — the documented install provenance.

A version that merely matches the locked `version` field is NOT proof of
the commit: an install whose commit cannot be proven from gitHead or the
pinned checkout is classified PIN-UNVERIFIED (exit 4) and excluded from
the official showdown output.

Usage:
    python3 repomix_adapter.py <repo> <token_budget> <out_dir> [--compress]

Writes:
    <out_dir>/repomix.txt   (the budget-capped, complete-file pack)
    <out_dir>/repomix-full.xml  (repomix's full output, for debugging)

Prints JSON on stdout:
    {"ok": true, "tool": "repomix", "tokens": <int>, "files": <int>,
     "artifact": "<path>", "compressed": <bool>, "pinned": "<commit>"}

Exit codes:
    0  success
    2  SKIPPED-UNINSTALLED — repomix is not installed; prints
       {"ok": false, "error": "SKIPPED-UNINSTALLED: ..."}
    3  PIN-MISMATCH — repomix is installed but demonstrably not the
       pinned commit (wrong gitHead / wrong checkout / wrong version);
       prints {"ok": false, "error": "PIN-MISMATCH: ..."}
    4  PIN-UNVERIFIED — repomix is installed and its version matches the
       lock but the commit cannot be proven (no gitHead, no pinned
       checkout); prints {"ok": false, "error": "PIN-UNVERIFIED: ..."}
    1  other failure

Installation (pinned; builds the exact commit, see README):
    git clone https://github.com/yamadashy/repomix.git ~/.scc-bench/repomix
    git -C ~/.scc-bench/repomix checkout e3b15a406ed78d8a463620a032a059ce911bfc0e
    npm install --prefix ~/.scc-bench/repomix
    npm run build --prefix ~/.scc-bench/repomix
    npm install -g ~/.scc-bench/repomix
"""

import html
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

LOCKED_REPOMIX_COMMIT = "e3b15a406ed78d8a463620a032a059ce911bfc0e"
LOCKED_REPOMIX_VERSION = "1.18.0"  # version declared at the pinned commit

# The documented pinned source checkout (built + globally installed).
# Overridable via SCC_REPOMIX_SRC_DIR (test seam: makes the pin check
# deterministic regardless of the local machine's checkout state).
PINNED_SOURCE_DIR = Path(os.environ.get("SCC_REPOMIX_SRC_DIR", str(Path.home() / ".scc-bench" / "repomix")))

FILE_RE = re.compile(r'<file path="([^"]+)"[^>]*>([\s\S]*?)</file>')


def estimate_tokens(text):
    # The single shared tokenizer for ALL variants (chars/4; the harness,
    # the scc side, and the aider adapter use the same rule).
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


def locate_repomix_package():
    """The installed repomix package directory (package.json parent), or
    None. Sources: `SCC_REPOMIX_PKG_DIR` (test/venv seam), the `repomix`
    binary on PATH (resolved through symlinks), the npm global root, and
    the npx cache."""
    cands = []
    override = os.environ.get("SCC_REPOMIX_PKG_DIR")
    if override:
        cands.append(Path(override))
    exe = shutil.which("repomix")
    if exe:
        cands.append(Path(exe).resolve())
    try:
        npm_root = subprocess.run(
            ["npm", "root", "-g"], capture_output=True, text=True, timeout=60
        )
        if npm_root.returncode == 0 and npm_root.stdout.strip():
            cands.append(Path(npm_root.stdout.strip()) / "repomix")
    except Exception:
        pass
    npx_cache = Path.home() / ".npm" / "_npx"
    if npx_cache.is_dir():
        cands.extend(sorted(npx_cache.glob("*/node_modules/repomix")))
    for cand in cands:
        pkg_json = cand if cand.name == "package.json" else cand / "package.json"
        if pkg_json.is_file():
            try:
                meta = json.loads(pkg_json.read_text())
            except ValueError:
                continue
            if meta.get("name") == "repomix":
                return pkg_json.parent
    return None


def verify_repomix_pin():
    """Raise PinMismatch / PinUnverified unless the installed repomix's
    COMMIT is proven against the lock. A version that merely matches the
    locked `version` is never proof of the commit: an install whose commit
    cannot be proven from gitHead or the pinned checkout is classified
    PIN-UNVERIFIED (the harness reports it as a distinct status, not a
    passing pin)."""
    pkg_dir = locate_repomix_package()
    if pkg_dir is None:
        raise PinMismatch("no repomix install found to verify against the lock")
    try:
        meta = json.loads((pkg_dir / "package.json").read_text())
    except (OSError, ValueError) as exc:
        raise PinMismatch(f"unreadable repomix package.json: {exc}") from exc
    version = meta.get("version")
    git_head = meta.get("gitHead")

    # Strongest: npm's recorded resolved commit for git-ref installs.
    if git_head:
        if git_head != LOCKED_REPOMIX_COMMIT:
            raise PinMismatch(
                f"installed repomix gitHead {git_head} != pinned {LOCKED_REPOMIX_COMMIT}"
            )
        return

    # The documented install: global install of the pinned source checkout.
    # Verify the source checkout's git HEAD is the locked commit AND the
    # installed version matches the source version (transitive pin: the
    # installed copy is built from the pinned checkout).
    try:
        out = subprocess.run(
            ["git", "-C", str(PINNED_SOURCE_DIR), "rev-parse", "HEAD"],
            capture_output=True, text=True, timeout=30,
        )
        src_head = out.stdout.strip() if out.returncode == 0 else None
        src_version = None
        src_pkg = PINNED_SOURCE_DIR / "package.json"
        if src_pkg.is_file():
            try:
                src_version = json.loads(src_pkg.read_text()).get("version")
            except ValueError:
                pass
    except Exception:
        src_head = None
        src_version = None
    if src_head == LOCKED_REPOMIX_COMMIT and src_version and src_version == version:
        return

    # Version-only coincidence is NOT commit proof. A version that differs
    # from the lock is a demonstrable mismatch; a matching version with an
    # unprovable commit is PIN-UNVERIFIED.
    if version != LOCKED_REPOMIX_VERSION:
        raise PinMismatch(
            f"installed repomix {version or '(unknown version)'} does not match the lock "
            f"(commit {LOCKED_REPOMIX_COMMIT}, version {LOCKED_REPOMIX_VERSION}); "
            "reinstall from the pinned source checkout (see benchmarks/external/README.md)"
        )
    raise PinUnverified(
        f"installed repomix version {version} matches the lock, but the commit cannot be "
        f"proven: no gitHead and no pinned checkout ({PINNED_SOURCE_DIR}) at "
        f"{LOCKED_REPOMIX_COMMIT}; refusing to treat a version-only match as a pin"
    )


class PinMismatch(Exception):
    pass


class PinUnverified(Exception):
    pass


NATIVE_CONTEXT_WINDOW = 200_000  # chars/4 feasibility ceiling for native mode


def run_repomix(argv):
    repo_abs = os.path.abspath(argv[1])
    budget = int(argv[2])
    out_dir = os.path.abspath(argv[3])
    compress = "--compress" in argv[4:]
    # --native (Part I): repomix's intended default behavior — the FULL
    # compressed pack, no shared-budget file-boundary cut. The artifact's
    # actual cost is reported; packs beyond a realistic context window are
    # reported infeasible rather than silently truncated.
    native = "--native" in argv[4:]

    if not os.path.isdir(repo_abs):
        return {"ok": False, "error": f"not a directory: {argv[1]}"}, 1
    os.makedirs(out_dir, exist_ok=True)
    argv_cmd = repomix_command(repo_abs, None, compress)
    if argv_cmd is None:
        return {
            "ok": False,
            "error": "SKIPPED-UNINSTALLED: neither `repomix` nor `npx` found on PATH "
                     f"(pinned commit {LOCKED_REPOMIX_COMMIT})",
        }, 2

    # Hard pin verification before running anything: the installed repomix
    # must match the locked COMMIT (no silent floating versions, no
    # version-only acceptance). An unprovable commit is PIN-UNVERIFIED,
    # never a passing pin.
    try:
        verify_repomix_pin()
    except PinUnverified as exc:
        return {"ok": False, "error": f"PIN-UNVERIFIED: {exc}"}, 4
    except PinMismatch as exc:
        return {"ok": False, "error": f"PIN-MISMATCH: {exc}"}, 3

    full_xml = os.path.join(out_dir, "repomix-full.xml")
    proc = subprocess.run(
        repomix_command(repo_abs, full_xml, compress), capture_output=True, text=True, timeout=600
    )
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

    # The final artifact's token estimate includes the `## File:` header
    # lines, so the cap is enforced on the concatenated artifact.
    if not native:
        kept = []
        total = 0
        for path, content in sections:
            header = f"## File: {path}\n"
            tokens = estimate_tokens(header + content)
            if kept and total + tokens > budget:
                break  # budget reached — stop at a file boundary
            kept.append((path, content))
            total += tokens
    else:
        kept = list(sections)
        packed_probe = "\n\n".join(f"## File: {p}\n{c}" for p, c in kept)
        if estimate_tokens(packed_probe) > NATIVE_CONTEXT_WINDOW // 1:
            return {
                "ok": False,
                "error": f"NATIVE-INFEASIBLE: full pack exceeds the {NATIVE_CONTEXT_WINDOW}-token context-window ceiling",
            }, 5
    if native:
        packed = "\n\n".join(f"## File: {path}\n{content}" for path, content in kept)
        total = estimate_tokens(packed)
        artifact = os.path.join(out_dir, "repomix-native.txt")
        with open(artifact, "w") as fh:
            fh.write(packed)
        print(json.dumps({
            "ok": True,
            "tool": "repomix",
            "tokens": total,
            "files": len(kept),
            "artifact": artifact,
            "compressed": compress,
            "pinned": LOCKED_REPOMIX_COMMIT,
            "mode": "native-default",
            "requested_budget": None,
            "actual_shared_tokens": total,
            "native_tool_budget_parameter": None,
        }))
        return None
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
        "pinned": LOCKED_REPOMIX_COMMIT,
        "mode": "equal-token",
        "requested_budget": budget,
        "actual_shared_tokens": total,
        "utilization": round(total / budget, 4) if budget else 0.0,
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
    if payload is not None:  # native mode already printed its own payload
        print(json.dumps(payload))
    return 0 if payload is None else code


if __name__ == "__main__":
    sys.exit(main(sys.argv))
