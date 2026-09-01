#!/usr/bin/env python3
"""Behavioral evaluator registry for the canonical writable benchmark tasks.

EVALUATOR CONTRACT (mission §25): every evaluator-backed coding task MUST
satisfy the four-way meta-contract:

    untouched fixture            -> FAIL
    comment-only / cosmetic patch -> FAIL
    plausible-but-wrong patch     -> FAIL
    known-correct implementation  -> PASS

Each evaluator runs in the ISOLATED post-agent repo copy and OBSERVES
BEHAVIOR wherever the fixture can actually run: the Python fixtures
execute real code (stdlib; the orthogonal tenacity decorator is stubbed),
asserting real state transitions. Where runtime execution cannot
reasonably be established (TypeScript fixtures carry no node_modules by
design — intentionally non-buildable benchmark data), the evaluator is an
EXPLICITLY CLASSIFIED structural check validating semantic shape (the
function that applies the change on the real call path), per §26.

The registry derives from benchmarks/tasks.json by task id (§28): one
canonical corpus, subsets selected by id.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import types
from pathlib import Path
from typing import Callable

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
TASKS_JSON = ROOT / "benchmarks" / "tasks.json"
FIXTURES = ROOT / "fixtures"

# task-id -> explicitly-classified structural evaluators (§26).
STRUCTURAL: set[str] = set()

Evaluator = Callable[[Path], bool]
EVALUATORS: dict[str, Evaluator] = {}


def evaluator(task_id: str, *, structural: bool = False):
    def deco(fn: Evaluator) -> Evaluator:
        if structural:
            STRUCTURAL.add(task_id)
        EVALUATORS[task_id] = fn
        return fn
    return deco


def load_tasks() -> list[dict]:
    with open(TASKS_JSON) as f:
        return json.load(f)["tasks"]


TENACITY_STUB = r"""
import types
tenacity = types.ModuleType("tenacity")
class _retry:
    def __init__(self, *a, **k): pass
    def __call__(self, fn): return fn
tenacity.retry = _retry
tenacity.wait_fixed = lambda *a, **k: None
tenacity.stop_after_attempt = lambda *a, **k: None
sys.modules["tenacity"] = tenacity
"""


def _py(repo: Path, code: str) -> tuple[int, str]:
    proc = subprocess.run(
        [sys.executable, "-c", code], cwd=str(repo),
        capture_output=True, text=True, timeout=120,
    )
    return proc.returncode, (proc.stdout or "") + (proc.stderr or "")


def _read(repo: Path, rel: str) -> str:
    p = repo / rel
    return p.read_text() if p.exists() else ""


# ---------------------------------------------------------------------------
# http-service-python — BEHAVIORAL (stdlib service layer exercised for real)
# ---------------------------------------------------------------------------

@evaluator("http-service.rename-transcript-field")
def eval_rename_transcript_field(repo: Path) -> bool:
    """The repository's returned dicts expose the renamed key
    (transcriptText) and no longer the old 'transcript' key, on a real
    seeded database. The untouched fixture crashes here (dict() over
    3-column tuples), so only a real rename passes."""
    code = r"""
import sqlite3, sys
sys.path.insert(0, "services")
from transcripts import TranscriptRepository

db = sqlite3.connect("transcripts.db")
db.execute("CREATE TABLE IF NOT EXISTS transcripts (id INTEGER PRIMARY KEY, raw_text TEXT, normalized_text TEXT)")
db.execute("DELETE FROM transcripts")
db.execute("INSERT INTO transcripts (raw_text, normalized_text) VALUES ('raw one', 'norm one')")
db.commit()

repo = TranscriptRepository()
rows = repo.find_all()
assert rows, "no rows returned"
for r in rows:
    assert "transcriptText" in r, f"renamed key missing: {r}"
    assert "transcript" not in r, f"old key still present: {r}"
print("OK")
"""
    rc, out = _py(repo, code)
    return rc == 0 and "OK" in out


@evaluator("http-service.transcript-normalization")
def eval_transcript_normalization(repo: Path) -> bool:
    """Normalization preserves the RAW text through a WORKING resolver:
    the untouched fixture returns only the resolver's output (raw is
    discarded), so it fails. A correct implementation keeps the raw text
    observable (returned alongside, or recoverable from the normalizer's
    result) even when the resolver succeeds. Failure-path preservation
    alone (the shipped behavior) is not enough."""
    code = r"""
import sys
sys.path.insert(0, "services")
from transcripts import Normalizer

class StrippingResolver:
    def resolve(self, raw_text: str) -> str:
        return raw_text.strip().upper()

n = Normalizer(resolver=StrippingResolver())
res = n.normalize("  Raw   Text  ")
text = str(res)
raw = "Raw   Text"
assert raw in text or "raw_text" in text or "raw" in text, \
    f"raw text not preserved through a working resolver: {res!r}"
print("OK")
"""
    rc, out = _py(repo, code)
    return rc == 0 and "OK" in out


@evaluator("http-service.health-check")
def eval_health_check(repo: Path) -> bool:
    """A health handler exists and returns a READINESS payload (status +
    at least one probe key). The handler is located by route signature and
    CALLED — the untouched fixture's {"status": "ok"} cannot pass because
    no probe key exists."""
    main = _read(repo, "main.py")
    if "/health" not in main:
        return False
    m = re.search(
        r"def\s+(health\w*)\s*\(\s*\)\s*(?:->\s*dict\s*)?:\s*\n((?:[ \t]+.*\n)+)", main)
    if not m:
        return False
    env: dict[str, object] = {}
    try:
        exec(f"def {m.group(1)}():\n" + m.group(2), env)  # noqa: S102 — fixture code
        payload = env[m.group(1)]()
    except Exception:
        return False
    if not isinstance(payload, dict) or "status" not in payload:
        return False
    return any(k in payload for k in ("uptime", "checks", "dependencies", "ok", "healthy"))


# ---------------------------------------------------------------------------
# py-queue-service — BEHAVIORAL (stdlib; tenacity stubbed, it is orthogonal)
# ---------------------------------------------------------------------------

@evaluator("py-queue.empty-messages")
def eval_empty_messages(repo: Path) -> bool:
    """consume({}) neither crashes nor stores junk; consume(real) still
    stores the incident end to end."""
    code = r"""
import sqlite3, sys
""" + TENACITY_STUB + r"""
db = sqlite3.connect("incidents.db")
try:
    db.execute("DROP TABLE incidents"); db.commit()
except sqlite3.OperationalError:
    pass
db.execute("CREATE TABLE IF NOT EXISTS incidents (id INTEGER PRIMARY KEY, text TEXT, severity TEXT)")
db.commit()

from consumer import consume

def rows_now():
    cur = sqlite3.connect("incidents.db").execute(
        "SELECT text, severity FROM incidents ORDER BY id")
    return list(cur)

try:
    consume({})
except Exception as e:
    raise AssertionError(f"empty message crashed consume: {e}")
if rows_now():
    raise AssertionError("empty message stored junk")

consume({"text": "smoke reported"})
rows = rows_now()
assert rows, "real message stored nothing"
assert rows[0][0] == "smoke reported", f"unexpected row: {rows}"
print("OK")
"""
    rc, out = _py(repo, code)
    return rc == 0 and "OK" in out


@evaluator("py-queue.classification-fallback")
def eval_classification_fallback(repo: Path) -> bool:
    """When classification fails, the incident is STILL STORED with the
    fallback severity (not lost, not 'error')."""
    code = r"""
import sqlite3, sys
""" + TENACITY_STUB + r"""
db = sqlite3.connect("incidents.db")
try:
    db.execute("DROP TABLE incidents"); db.commit()
except sqlite3.OperationalError:
    pass
db.execute("CREATE TABLE IF NOT EXISTS incidents (id INTEGER PRIMARY KEY, text TEXT, severity TEXT)")
db.commit()

import processor
from store import IncidentStore

def model_down(text):
    raise RuntimeError("model down")
processor.classify = model_down
processor.process_incident({"text": "fire reported"}, IncidentStore())
rows = list(sqlite3.connect("incidents.db").execute(
    "SELECT text, severity FROM incidents ORDER BY id"))
assert rows, "fallback path dropped the incident"
assert rows[0][1] not in ("error", None, ""), f"bad severity: {rows[0]}"
# The task is "add a fallback when classification fails": a hardcoded
# literal is the fixture's shipped state, so the evaluator demands a
# named/configured fallback mechanism in the source.
src = open("processor.py").read()
import re as _re
named = bool(_re.search(
    r"(DEFAULT_SEVERITY|FALLBACK[_A-Z]*|fallback_severity|SEVERITY_FALLBACK)\s*=", src, _re.IGNORECASE))
assert named, "fallback must be a named mechanism (e.g. DEFAULT_SEVERITY = ...), not the shipped hardcoded literal"
print("OK")
"""
    rc, out = _py(repo, code)
    return rc == 0 and "OK" in out


@evaluator("py-queue.store-changes")
def eval_store_changes(repo: Path) -> bool:
    """The store persists and reads back real rows in order after the
    change. Schema ownership belongs to the repo under test (the task IS
    a storage change): the evaluator only clears the database and then
    exercises save/recent — a correct implementation bootstraps whatever
    table shape it needs."""
    code = r"""
import sqlite3, sys
db = sqlite3.connect("incidents.db")
try:
    db.execute("DROP TABLE incidents"); db.commit()
except sqlite3.OperationalError:
    pass
db.close()
from store import IncidentStore
s = IncidentStore()
s.save_incident("a", "high")
s.save_incident("b", "low")
rows = s.recent()
assert isinstance(rows, list) and rows, f"recent() must return rows, got: {rows!r}"
first = rows[0]
text = first.get("text") if isinstance(first, dict) else (first[1] if isinstance(first, (list, tuple)) and len(first) > 1 else None)
assert text == "b", f"most-recent-first order broken: {rows}"
assert "severity" in str(first), f"severity not persisted: {rows}"
print("OK")
"""
    rc, out = _py(repo, code)
    return rc == 0 and "OK" in out


# ---------------------------------------------------------------------------
# TypeScript fixtures — CLASSIFIED STRUCTURAL (§26): no node_modules by
# design; these verify the change's semantic shape on the real call path,
# not a bare word grep, and each satisfies the four-way contract.
# ---------------------------------------------------------------------------

@evaluator("queue-worker.asr-retry", structural=True)
def eval_asr_retry(repo: Path) -> bool:
    """The ASR CALL PATH (src/ingest.ts) gains APPLIED retry logic around
    the transcribe call — a real loop/catch wrapping the call, not the
    pre-existing client.ts decorator, not an uncalled helper, not a
    comment."""
    ingest = _read(repo, "src/ingest.ts")
    if not ingest or "transcribe" not in ingest:
        return False
    # Applied: a loop containing an attempt bound AND the transcribe call
    # INSIDE it, or a try/catch directly around the transcribe call.
    applied = bool(re.search(
        r"for\s*\([^)]*attempt[^)]*\)\s*\{[\s\S]{0,300}await\s+transcribe", ingest))
    if not applied:
        m = re.search(r"try\s*\{[\s\S]{0,200}await\s+transcribe[\s\S]{0,200}\}\s*catch", ingest)
        applied = bool(m)
    return applied


@evaluator("queue-worker.street-vocabulary", structural=True)
def eval_street_vocabulary(repo: Path) -> bool:
    """The resolver APPLIES a department vocabulary map in its resolution
    path: the map exists AND a .replace(/.includes(/.get( application
    appears in the resolver body."""
    resolver = _read(repo, "src/geo/resolver.ts")
    if not resolver:
        return False
    has_map = bool(re.search(r"Map\s*<|VOCAB", resolver, re.IGNORECASE))
    applied = bool(re.search(r"\.(replace|includes|get|has)\s*\(", resolver))
    return has_map and applied


@evaluator("ts-api-web.pagination", structural=True)
def eval_pagination(repo: Path) -> bool:
    combined = _read(repo, "service/users.ts") + "\n" + _read(repo, "server.ts")
    if not combined.strip():
        return False
    # The param must be a FUNCTION PARAMETER (in a signature) or a query
    # read — not a bare const declaration (the wrong patch's
    # `const DEFAULT_PAGE = 1;` must not pass).
    in_signature = bool(re.search(
        r"function\s+\w+\s*\([^)]*(page|limit|offset)[^)]*\)", combined, re.IGNORECASE))
    async_sig = bool(re.search(
        r"async\s+\w+\s*\(([^)]*(page|limit|offset)[^)]*)\)", combined, re.IGNORECASE))
    arrow_sig = bool(re.search(
        r"(page|limit|offset)\s*(:|\?)\s*number", combined, re.IGNORECASE))
    reads_param = in_signature or async_sig or arrow_sig
    # AND the pagination param must reach a data call (findMany/query/
    # list/slice/where) — threading, not just presence in a signature.
    threads = bool(re.search(
        r"(findMany|where|skip|take|slice|LIMIT|OFFSET)\s*\([^)]*(page|limit|offset)|"
        r"(page|limit|offset)[^\n]{0,80}(findMany|where|skip|take)", combined, re.IGNORECASE))
    return reads_param and threads





def evaluate(task_id: str, repo: Path) -> bool:
    fn = EVALUATORS.get(task_id)
    if fn is None:
        raise KeyError(f"no evaluator registered for {task_id!r}")
    return fn(repo)


def is_structural(task_id: str) -> bool:
    return task_id in STRUCTURAL


if __name__ == "__main__":
    task_id, repo = sys.argv[1], Path(sys.argv[2])
    ok = evaluate(task_id, repo)
    print(json.dumps({"task": task_id, "repo": str(repo),
                      "success": ok, "structural": is_structural(task_id)}))
    sys.exit(0 if ok else 1)
