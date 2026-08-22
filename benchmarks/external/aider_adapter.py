#!/usr/bin/env python3
"""Aider RepoMap adapter (Wave 15 external benchmark suite).

Invokes the PINNED aider repo-map (benchmarks/external-lock.json:
Aider-AI/aider @ 5dc9490bb) through its real RepoMap API and writes the map
for a repository under an equal-token budget. The installed aider MUST be
the pinned commit — the adapter verifies the install metadata (the
pip direct_url.json `vcs_info.commit_id` from the git install, or the
package's own `.git` HEAD for editable installs) and hard-errors with
`PIN-MISMATCH` (exit 3) on any mismatch or unverifiable install. No silent
floating versions.

Task personalization (fair Aider-vs-task-SCC): the goal's mentioned
identifiers (simple word tokens) are passed as `mentioned_idents` to
`RepoMap.get_repo_map` (aider's personalization input — aider/repomap.py
at the pinned commit, signature `get_repo_map(chat_files, other_files,
mentioned_fnames=None, mentioned_idents=None, force_refresh=False)`), the
same goal the SCC task variants personalize with.

Usage:
    python3 aider_adapter.py <repo> <token_budget> <out_dir> [--goal "<goal>"]

Writes:
    <out_dir>/aider-map.txt

Prints JSON on stdout:
    {"ok": true, "tool": "aider", "tokens": <int>, "files": <int>,
     "artifact": "<path>", "pinned": "<commit>"}

Exit codes:
    0  success
    2  SKIPPED-UNINSTALLED — aider (or its python deps) is not installed;
       prints {"ok": false, "error": "SKIPPED-UNINSTALLED: ..."} and the
       harness reports the variant as SKIPPED-UNINSTALLED.
    3  PIN-MISMATCH — aider is installed but does not match the pinned
       commit (or the installed commit is unverifiable); prints
       {"ok": false, "error": "PIN-MISMATCH: ..."}.
    1  other failure

Installation (pinned):
    python3.12 -m venv ~/.scc-bench-venv
    ~/.scc-bench-venv/bin/pip install \
        "git+https://github.com/Aider-AI/aider.git@5dc9490bb35f9729ef2c95d00a19ccd30c26339c"
"""

import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

LOCKED_AIDER_COMMIT = "5dc9490bb35f9729ef2c95d00a19ccd30c26339c"

# Grammar words — never identifiers. Everything else in the goal is a
# candidate identifier token for aider's personalization.
GOAL_STOPWORDS = frozenset(
    """a an the to of for on in with from into that this not but you our all
    can has had its who what and are was were is be it as at by or if then
    than so do does did have been being add make use set up out about over
    under between through during before after above below again further then
    once here there when where why how both each few more most other some such
    no nor only own same too very just should now will would could may might
    must shall
    change fix rename update refactor implement handle check test error
    handling field response request value new old one two three code file
    line call return param argument method function class module package
    version build run start stop get put post delete create remove
""".split()
)


class AiderMissing(Exception):
    pass


class PinMismatch(Exception):
    pass


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
    # Deterministic chars/4 heuristic — the single shared tokenizer for ALL
    # variants (harness, adapters, and the scc side use the same rule), so
    # equal-token budgets mean the same thing everywhere.
    if not text:
        return 0
    return max(1, len(text) // 4)


def installed_aider_commit(site_packages=None):
    """Return the installed aider commit, or None when no aider install is
    found. Sources, in order: pip's direct_url.json (git installs record
    `vcs_info.commit_id`), then the package's own `.git` HEAD (editable
    installs). `site_packages` overrides the search roots (used by the
    integration tests; defaults to the running interpreter's site-packages
    plus the aider package's parent)."""
    roots = []
    if site_packages:
        roots.append(Path(site_packages))
    try:
        import aider
        roots.append(Path(aider.__file__).resolve().parent.parent)
    except Exception:
        pass
    for root in roots:
        if not root.is_dir():
            continue
        for dist in root.glob("*.dist-info"):
            direct_url = dist / "direct_url.json"
            if not direct_url.is_file():
                continue
            try:
                info = json.loads(direct_url.read_text())
            except ValueError:
                continue
            commit = (info.get("vcs_info") or {}).get("commit_id")
            if "Aider-AI/aider" in info.get("url", "") and commit:
                return commit
    # editable installs: the package source is a git checkout
    try:
        import aider
        pkg_dir = Path(aider.__file__).resolve().parent
        if (pkg_dir / ".git").exists():
            out = subprocess.run(
                ["git", "-C", str(pkg_dir), "rev-parse", "HEAD"],
                capture_output=True, text=True, timeout=30,
            )
            if out.returncode == 0 and out.stdout.strip():
                return out.stdout.strip()
    except Exception:
        pass
    return None


IDENT_STOPWORDS = GOAL_STOPWORDS


def mentioned_idents(goal):
    """Extract mentioned identifiers from the task goal, PRESERVING ORIGINAL
    CASE. The pinned aider matches `mentioned_idents` exactly (case-
    sensitive) against file/symbol tokens when personalizing the map, so
    lowercasing here silently suppressed its boost for PascalCase /
    camelCase mentions ("IncidentEngine" → "incidentengine" matched
    nothing). Each token is emitted in its original spelling; a lowercase
    variant is ADDED ONLY for mixed-case tokens (so both `IncidentEngine`
    and `incidentengine` can hit), never as a replacement. Grammar words
    are dropped; `--flag` prefixes are stripped; likely-identifier shapes
    (snake_case, camelCase, PascalCase, dotted.names, paths, filenames)
    pass through.
    """
    out = []
    seen = set()
    # Split on whitespace/quotes first so path-ish tokens (src/engine.ts,
    # services/transcripts.py) survive whole, then split each chunk on
    # non-[A-Za-z0-9_.-] for bare identifiers.
    chunks = re.split(r"[\s`'\"]+", goal or "")
    for chunk in chunks:
        tok = chunk.strip().rstrip(".,;:!?)")
        if not tok:
            continue
        if tok.startswith("-"):
            # CLI flag: aider personalizes on identifiers, not flags.
            continue
        candidates = [tok]
        if not re.fullmatch(r"[A-Za-z0-9_./\-]+", tok):
            # punctuation-heavy chunk: fall back to alnum splitting for the
            # inner words (original behavior), keeping case
            candidates = [
                w for w in re.split(r"[^A-Za-z0-9_]+", tok) if w
            ]
        for cand in candidates:
            forms = [cand]
            if cand != cand.lower():
                # mixed case: keep the original spelling first-class and add
                # the lowercase twin AFTER it (aider matches exactly, so both
                # spellings can hit; the original always leads)
                forms.append(cand.lower())
            for idx, form in enumerate(forms):
                low = form.lower()
                if len(form) < 3 or low in IDENT_STOPWORDS:
                    continue
                if form in seen:
                    continue
                if idx > 0 and low in seen:
                    # an all-lowercase token with this spelling was already
                    # emitted — the twin would be a duplicate
                    continue
                seen.add(form)
                if form == low:
                    seen.add(low)
                out.append(form)
    return out


def mentioned_fnames(goal):
    """Paths/filenames literally present in the task goal: any whitespace-
    delimited chunk containing a `/` or looking like `name.<ext>` with a
    plausible source extension. Only strings ACTUALLY IN the task are
    returned — ground truth is never inferred here."""
    exts = (
        ".py", ".ts", ".tsx", ".js", ".jsx", ".rs", ".go", ".java", ".rb",
        ".c", ".h", ".cpp", ".hpp", ".cs", ".php", ".swift", ".kt", ".md",
        ".json", ".yaml", ".yml", ".toml",
    )
    low = goal or ""
    out = []
    seen = set()
    for chunk in re.split(r"[\s`'\"]+", low):
        tok = chunk.strip().lstrip("-").rstrip(".,;:!?)")
        if len(tok) < 3 or tok in seen:
            continue
        if "/" in tok or tok.lower().endswith(exts):
            seen.add(tok)
            out.append(tok)
    return out


def build_repomap(repo_abs, budget, goal, site_packages=None):
    """Construct the pinned aider RepoMap for the budget. Returns
    `(map_text, map_tokens_used)` — the map_tokens the equal-token search
    actually landed on (the harness records it as
    `native_tool_budget_parameter`). Raises AiderMissing / PinMismatch."""
    commit = installed_aider_commit(site_packages=site_packages)
    if commit is None:
        raise AiderMissing(
            "aider install not found (no pip direct_url.json commit_id / package .git HEAD)"
        )
    if commit != LOCKED_AIDER_COMMIT:
        raise PinMismatch(
            f"installed aider commit {commit} != pinned {LOCKED_AIDER_COMMIT} "
            "(reinstall with the pinned git ref; see benchmarks/external/README.md)"
        )

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

    other_files = src_files(repo_abs)
    idents = set(mentioned_idents(goal))
    fnames = set(mentioned_fnames(goal))

    def gen(map_tokens):
        """One aider map at a given map_tokens. Aider sizes the map with
        its OWN model tokenizer (its binary search targets ±15% of
        map_tokens), so map_tokens alone does not make the shared chars/4
        budget equal-token — the caller searches over it."""
        try:
            rm = RepoMap(
                map_tokens=map_tokens,
                root=repo_abs,
                main_model=model,
                io=io,
                refresh="manual",
            )
        except TypeError:
            rm = RepoMap(map_tokens, repo_abs, model, io)
        try:
            return rm.get_repo_map(
                [], other_files, mentioned_idents=idents, mentioned_fnames=fnames
            ) or ""
        except TypeError:
            # Older pinned signature without the mentioned_fnames kwarg.
            return rm.get_repo_map([], other_files, mentioned_idents=idents) or ""

    # EQUAL-TOKEN mode (Part H): the shared chars/4 estimator governs.
    # Search the largest map_tokens whose generated map fits the budget
    # under estimate_tokens — aider's internal tokenizer may disagree with
    # chars/4 in either direction, so a single map_tokens=budget call can
    # over- OR under-shoot. Geometric ramp + binary search; the generated
    # map itself always fits (never mid-truncated).
    map_tokens = budget
    text = gen(map_tokens)
    if estimate_tokens(text) > budget:
        hi = map_tokens
        lo = 0
        while lo < hi:
            mid = (lo + hi) // 2
            candidate = gen(max(1, mid))
            if estimate_tokens(candidate) <= budget:
                text = candidate
                map_tokens = max(1, mid)
                lo = mid + 1
            else:
                hi = mid
    return text, map_tokens


def build_repomap_native(repo_abs, goal, site_packages=None):
    """NATIVE-DEFAULT mode (Part I): aider's intended default behavior —
    RepoMap constructed WITHOUT a map_tokens override (aider's own
    default), personalized with the goal's idents/fnames as usual. The
    artifact's actual cost is reported, never normalized; do not use this
    mode to claim per-token superiority. Returns `(map_text,
    map_tokens_used)` where the second element is `None` (aider default)."""
    commit = installed_aider_commit(site_packages=site_packages)
    if commit is None:
        raise AiderMissing("aider install not found")
    if commit != LOCKED_AIDER_COMMIT:
        raise PinMismatch(
            f"installed aider commit {commit} != pinned {LOCKED_AIDER_COMMIT}"
        )
    try:
        from aider.io import InputOutput  # noqa: F401
        from aider.models import Model
        from aider.repomap import RepoMap
    except ImportError as exc:
        raise AiderMissing(f"aider python package not importable: {exc}") from exc
    try:
        model = Model("gpt-4o")
    except Exception:
        model = None
    io = InputOutput(pretty=False, yes=True)
    try:
        rm = RepoMap(root=repo_abs, main_model=model, io=io, refresh="manual")
    except TypeError:
        rm = RepoMap(None, repo_abs, model, io)
    other_files = src_files(repo_abs)
    idents = set(mentioned_idents(goal))
    fnames = set(mentioned_fnames(goal))
    try:
        text = rm.get_repo_map(
            [], other_files, mentioned_idents=idents, mentioned_fnames=fnames
        ) or ""
    except TypeError:
        text = rm.get_repo_map([], other_files, mentioned_idents=idents) or ""
    return text, None

def main(argv):
    # --native (Part I): aider's OWN default configuration — no shared
    # budget, no search; RepoMap is constructed with aider's default
    # map_tokens and the artifact's actual cost is REPORTED, never
    # normalized. Scores from this mode must not be read per-token.
    native = "--native" in argv[4:]
    if "--native" in argv[4:]:
        argv = [a for a in argv if a != "--native"]
    if len(argv) not in (4, 6) or (len(argv) == 6 and argv[4] != "--goal"):
        print(
            json.dumps(
                {
                    "ok": False,
                    "error": (
                        f"usage: {argv[0]} <repo> <token_budget> <out_dir> "
                        f"[--goal '<task goal>'] (pinned aider commit {LOCKED_AIDER_COMMIT})"
                    ),
                }
            )
        )
        return 1

    repo, budget_s, out_dir = argv[1], argv[2], argv[3]
    goal = argv[5] if len(argv) == 6 else ""
    # Test/venv seam: override the site-packages search root for the pin
    # check (defaults to the running interpreter's installs).
    site_packages = os.environ.get("SCC_AIDER_SITE_PACKAGES") or None
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
            if native:
                map_text, map_tokens_used = build_repomap_native(
                    repo_abs, goal, site_packages=site_packages
                )
            else:
                map_text, map_tokens_used = build_repomap(
                    repo_abs, budget, goal, site_packages=site_packages
                )
    except AiderMissing as exc:
        print(json.dumps({"ok": False, "error": f"SKIPPED-UNINSTALLED: {exc}"}))
        return 2
    except PinMismatch as exc:
        print(json.dumps({"ok": False, "error": f"PIN-MISMATCH: {exc}"}))
        return 3
    except Exception as exc:  # aider internals can throw on odd repos
        print(json.dumps({"ok": False, "error": f"aider repomap failed: {exc}"}))
        return 1

    artifact = os.path.join(out_dir, "aider-map.txt")
    with open(artifact, "w") as fh:
        fh.write(map_text)

    actual = estimate_tokens(map_text)
    print(
        json.dumps(
            {
                "ok": True,
                "tool": "aider",
                "tokens": actual,
                "files": len(src_files(repo_abs)),
                "artifact": artifact,
                "pinned": LOCKED_AIDER_COMMIT,
                "mode": "native-default" if native else "equal-token",
                "requested_budget": budget,
                "actual_shared_tokens": actual,
                "native_tool_budget_parameter": map_tokens_used,
                "utilization": round(actual / budget, 4) if budget else 0.0,
                "mode": "equal-token",
            }
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
