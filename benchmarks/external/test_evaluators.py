#!/usr/bin/env python3
"""Evaluator four-way meta-contract tests (mission §25, §29).

For EVERY evaluator-backed writable task, the evaluator must satisfy:

    untouched fixture            -> FAIL
    comment-only / cosmetic patch -> FAIL
    plausible-but-wrong patch     -> FAIL
    known-correct implementation  -> PASS

A plausible-but-wrong patch is a patch a lazy agent might write: it
mentions the right words (retry, page, health) but does NOT implement the
behavior (a comment, a dead constant, a no-op function, a marker renamed
in only one place). If any evaluator passes such a patch, the benchmark
measures nothing.

These are benchmark-infrastructure tests: they run in CI (§36) BEFORE any
agent experiment.
"""

from __future__ import annotations

import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from evaluators import EVALUATORS, STRUCTURAL, load_tasks, FIXTURES  # noqa: E402

WRITABLE_TASK_IDS = [t["id"] for t in load_tasks()
                     if t["id"] in EVALUATORS and t["id"] != "http-service.monorepo-transcripts"]


def fresh_copy(repo_name: str) -> Path:
    tmp = Path(tempfile.mkdtemp(prefix="ev-meta-"))
    root = tmp / repo_name
    shutil.copytree(FIXTURES / repo_name, root,
                    ignore=shutil.ignore_patterns(".scc", ".aider*"))
    return root


def patch(repo: Path, rel: str, find: str, replace: str) -> None:
    """Apply a deterministic text patch to a file in the copy."""
    p = repo / rel
    text = p.read_text()
    assert find in text, f"patch anchor missing in {rel}: {find!r}"
    p.write_text(text.replace(find, replace))


class EvaluatorMetaContract(unittest.TestCase):
    """Every registered evaluator passes the four-way contract."""

    def _assert_contract(self, task_id: str, repo_name: str):
        from evaluators import evaluate
        with self.subTest(task=task_id):
            # 1. Untouched fixture FAILS.
            repo = fresh_copy(repo_name)
            self.assertFalse(evaluate(task_id, repo),
                             f"{task_id}: evaluator PASSED on the untouched fixture — it measures nothing")

            # 2/3/4 are task-specific patches defined below.
            wrong = WRONG_PATCHES.get(task_id)
            right = KNOWN_GOOD_PATCHES.get(task_id)
            self.assertIsNotNone(wrong, f"{task_id}: no plausible-wrong patch defined for the meta test")
            self.assertIsNotNone(right, f"{task_id}: no known-good patch defined for the meta test")

            # 2. Comment-only / cosmetic patch FAILS.
            repo = fresh_copy(repo_name)
            apply_cosmetic(repo)
            self.assertFalse(evaluate(task_id, repo),
                             f"{task_id}: evaluator PASSED a comment-only patch")

            # 3. Plausible-but-wrong patch FAILS.
            repo = fresh_copy(repo_name)
            wrong(repo)
            self.assertFalse(evaluate(task_id, repo),
                             f"{task_id}: evaluator PASSED a plausible-but-wrong patch")

            # 4. Known-correct implementation PASSES.
            repo = fresh_copy(repo_name)
            right(repo)
            self.assertTrue(evaluate(task_id, repo),
                            f"{task_id}: evaluator FAILED a known-correct implementation")


# ---------------------------------------------------------------------------
# Per-task patch generators. The WRONG patch is what a lazy agent writes:
# the right vocabulary, applied nowhere (or applied off the real path).
# ---------------------------------------------------------------------------

def cosmetic_comment(repo: Path, rel: str, comment: str) -> None:
    p = repo / rel
    text = p.read_text()
    p.write_text(f"# {comment}\n" + text if rel.endswith(".py")
                 else f"// {comment}\n" + text)


def apply_cosmetic(repo: Path) -> None:
    """A pure comment added to each fixture's entry file."""
    for rel, c in [("main.py", "todo: implement"), ("consumer.py", "todo: implement"),
                   ("src/ingest.ts", "todo: implement"), ("service/users.ts", "todo: implement")]:
        p = repo / rel
        if p.exists():
            text = p.read_text()
            prefix = "#" if rel.endswith(".py") else "//"
            p.write_text(f"{prefix} {c}\n" + text)


def wrong_rename_transcript_field(repo: Path) -> None:
    # Right word, wrong place: a renamed VARIABLE in the repository, but
    # the returned dicts still carry the old key.
    patch(repo, "services/transcripts.py",
          "SELECT id, raw_text, normalized_text FROM transcripts",
          "SELECT id AS transcriptText, raw_text, normalized_text FROM transcripts")


def good_rename_transcript_field(repo: Path) -> None:
    # Rows are positional tuples; build the renamed response dict explicitly.
    patch(repo, "services/transcripts.py",
          "return [dict(r) for r in rows]",
          'return [{"id": r[0], "transcriptText": r[2]} for r in rows]')


def wrong_normalization(repo: Path) -> None:
    # Mentions preservation but strips HARDER (still discards raw).
    patch(repo, "services/transcripts.py",
          "return self.resolver.resolve(raw_text)",
          'return self.resolver.resolve(raw_text).strip().upper()')


def good_normalization(repo: Path) -> None:
    # Known-good: normalization returns both normalized and raw text so
    # the raw stays observable through a working resolver.
    patch(repo, "services/transcripts.py",
          '        try:\n            return self.resolver.resolve(raw_text)\n        except Exception:\n            return raw_text',
          '        try:\n            out = self.resolver.resolve(raw_text)\n            return {"normalized": out, "raw_text": raw_text}\n        except Exception:\n            return raw_text')
def wrong_health(repo: Path) -> None:
    # A health endpoint that returns the same fixture payload (no
    # readiness info) — plausible, useless.
    patch(repo, "main.py",
          '@app.get("/health")\ndef health() -> dict:\n    return {"status": "ok"}',
          '@app.get("/health")\ndef health() -> dict:\n    # readiness payload\n    return {"status": "ok"}')


def good_health(repo: Path) -> None:
    patch(repo, "main.py",
          '@app.get("/health")\ndef health() -> dict:\n    return {"status": "ok"}',
          '@app.get("/health")\ndef health() -> dict:\n    return {"status": "ok", "uptime": 42, "checks": {"db": "up"}}')


def wrong_empty_messages(repo: Path) -> None:
    # Swallows the message with a comment claiming tolerance but writes
    # the empty incident anyway (or crashes) — no, simpler: it crashes.
    patch(repo, "consumer.py",
          "def consume(message: dict) -> None:",
          "def consume(message: dict) -> None:\n    # tolerate empty messages")


def good_empty_messages(repo: Path) -> None:
    patch(repo, "consumer.py",
          "def consume(message: dict) -> None:\n    store = IncidentStore()",
          "def consume(message: dict) -> None:\n    if not message or not message.get(\"text\"):\n        return\n    store = IncidentStore()")


def wrong_classification_fallback(repo: Path) -> None:
    # Catches the error but re-raises it (mentions fallback, drops the row).
    patch(repo, "processor.py",
          '    except Exception:\n        severity = "unknown"  # fallback: default severity',
          '    except Exception:\n        severity = "unknown"  # fallback: default severity\n    if severity == "unknown":\n        raise')


def good_classification_fallback(repo: Path) -> None:
    # The fixture already implements the fallback correctly; the
    # known-good implementation makes the fallback severity a named
    # module-level mechanism while preserving behavior.
    patch(repo, "processor.py",
          "import tenacity\nfrom store import IncidentStore",
          "import tenacity\nfrom store import IncidentStore\n\nDEFAULT_SEVERITY = \"unknown\"  # named fallback mechanism")
    patch(repo, "processor.py",
          '    except Exception:\n        severity = "unknown"  # fallback: default severity',
          '    except Exception:\n        severity = DEFAULT_SEVERITY')


def wrong_asr_retry(repo: Path) -> None:
    # A retry LOOP that retries nothing (empty body) on a NEW function
    # nobody calls — right words, off the call path.
    p = repo / "src/ingest.ts"
    p.write_text(p.read_text() + """
// retry handling
export async function retryWrapper<T>(fn: () => Promise<T>): Promise<T> {
  for (let i = 0; i < 3; i++) { /* attempt */ }
  return fn();
}
""")


def good_asr_retry(repo: Path) -> None:
    patch(repo, "src/ingest.ts",
          "  const raw = await transcribe(message.value);",
          """  let raw: string = "";
  for (let attempt = 0; attempt < 3; attempt++) {
    try { raw = await transcribe(message.value); break; }
    catch (e) { if (attempt === 2) throw e; }
  }""")


def wrong_street_vocabulary(repo: Path) -> None:
    # Defines the vocabulary map but never applies it in resolution.
    p = repo / "src/geo/resolver.ts"
    p.write_text(p.read_text() + """

const DEPARTMENT_VOCAB = new Map<string, string>();
""")


def good_street_vocabulary(repo: Path) -> None:
    patch(repo, "src/geo/resolver.ts",
          "  return text;",
          """  const DEPARTMENT_VOCAB: Map<string, string> = new Map([["main street", "dept 5"]]);
  let normalized = text;
  for (const [from, to] of DEPARTMENT_VOCAB) {
    if (normalized.includes(from)) { normalized = normalized.replace(from, to); break; }
  }
  return normalized;""")


def wrong_pagination(repo: Path) -> None:
    # Adds the word page in a comment/constant, threads nothing.
    p = repo / "service/users.ts"
    p.write_text(p.read_text() + "\n// page/pageSize pagination TODO\nconst DEFAULT_PAGE = 1;\n")


def good_pagination(repo: Path) -> None:
    # Real pagination on the fixture's actual data path: listUsers gains
    # page/pageSize parameters and threads them into the db call
    # (prisma-style take/skip on findMany).
    patch(repo, "service/users.ts",
          "export async function listUsers(): Promise<Array<{ id: string; name: string }>> {\n"
          "  const rows = await db.users.findMany({ select: { id: true, name: true } });\n"
          "  return rows;\n"
          "}",
          "export async function listUsers(page: number = 1, pageSize: number = 20): "
          "Promise<Array<{ id: string; name: string }>> {\n"
          "  const rows = await db.users.findMany({\n"
          "    select: { id: true, name: true },\n"
          "    take: pageSize,\n"
          "    skip: (page - 1) * pageSize,\n"
          "  });\n"
          "  return rows;\n"
          "}")


def wrong_store_changes(repo: Path) -> None:
    # Renames the method (mentions store changes) breaking the reader.
    patch(repo, "store.py", "def recent(", "def recent_changes(")


def good_store_changes(repo: Path) -> None:
    # A real storage change: the store bootstraps its own (new) schema
    # including a created_at column, inserts through it, and recent()
    # builds dicts from positional rows (the shipped dict(r) crashes).
    patch(repo, "store.py",
          '        self.db.execute(\n            "INSERT INTO incidents (text, severity) VALUES (?, ?)",\n            (text, severity),\n        )',
          '        self._ensure_schema()\n        self.db.execute(\n            "INSERT INTO incidents (text, severity, created_at) VALUES (?, ?, ?)",\n            (text, severity, time.time()),\n        )')
    patch(repo, "store.py",
          "    def __init__(self):\n        self.db = sqlite3.connect(\"incidents.db\")",
          "    def __init__(self):\n        self.db = sqlite3.connect(\"incidents.db\")\n        self._ensure_schema()\n\n    def _ensure_schema(self):\n        self.db.execute(\n            \"CREATE TABLE IF NOT EXISTS incidents (id INTEGER PRIMARY KEY, text TEXT, severity TEXT, created_at REAL)\"\n        )\n        self.db.commit()")
    patch(repo, "store.py",
          "        return [dict(r) for r in rows]",
          '        return [{"id": r[0], "text": r[1], "severity": r[2]} for r in rows]')
    patch(repo, "store.py",
          "import sqlite3",
          "import sqlite3\nimport time")


REPO_OF = {
    "http-service.rename-transcript-field": "http-service-python",
    "http-service.transcript-normalization": "http-service-python",
    "http-service.health-check": "http-service-python",
    "py-queue.empty-messages": "py-queue-service",
    "py-queue.classification-fallback": "py-queue-service",
    "py-queue.store-changes": "py-queue-service",
    "queue-worker.asr-retry": "queue-worker-ts",
    "queue-worker.street-vocabulary": "queue-worker-ts",
    "ts-api-web.pagination": "ts-api-web",
}

WRONG_PATCHES = {
    "http-service.rename-transcript-field": wrong_rename_transcript_field,
    "http-service.transcript-normalization": wrong_normalization,
    "http-service.health-check": wrong_health,
    "py-queue.empty-messages": wrong_empty_messages,
    "py-queue.classification-fallback": wrong_classification_fallback,
    "py-queue.store-changes": wrong_store_changes,
    "queue-worker.asr-retry": wrong_asr_retry,
    "queue-worker.street-vocabulary": wrong_street_vocabulary,
    "ts-api-web.pagination": wrong_pagination,
}

KNOWN_GOOD_PATCHES = {
    "http-service.rename-transcript-field": good_rename_transcript_field,
    "http-service.transcript-normalization": good_normalization,
    "http-service.health-check": good_health,
    "py-queue.empty-messages": good_empty_messages,
    "py-queue.classification-fallback": good_classification_fallback,
    "py-queue.store-changes": good_store_changes,
    "queue-worker.asr-retry": good_asr_retry,
    "queue-worker.street-vocabulary": good_street_vocabulary,
    "ts-api-web.pagination": good_pagination,
}


class TestFourWayContract(EvaluatorMetaContract):
    def test_all_writable_evaluators(self):
        for task_id in WRITABLE_TASK_IDS:
            repo_name = REPO_OF[task_id]
            self._assert_contract(task_id, repo_name)

    def test_registry_covers_canonical_writable_ids(self):
        # Every task the writable matrix can select has an evaluator
        # (§28: subsets derive by id; unknown id -> KeyError, never a
        # silent pass).
        from evaluators import EVALUATORS as E
        self.assertTrue(len(E) >= len(WRITABLE_TASK_IDS))

    def test_structural_classification_recorded(self):
        from evaluators import STRUCTURAL as S
        self.assertIn("queue-worker.asr-retry", S)
        self.assertIn("queue-worker.street-vocabulary", S)
        self.assertIn("ts-api-web.pagination", S)
        self.assertNotIn("py-queue.empty-messages", S,
                         "py-queue evaluators are behavioral — must not be classified structural")


if __name__ == "__main__":
    unittest.main(verbosity=2)


class TestMatrixWiring(unittest.TestCase):
    """The writable matrix derives from the canonical corpus and uses the
    registry evaluators (mission §25, §28)."""

    def test_scoped_tasks_derive_from_canonical(self):
        import run_write_matrix as rwm
        import evaluators as ev
        canonical_ids = {t["id"] for t in ev.load_tasks()}
        for t in rwm.SCOPED_TASKS:
            self.assertIn(t["id"], canonical_ids,
                          f"matrix task {t['id']} not in canonical tasks.json")
            self.assertIn(t["id"], ev.EVALUATORS,
                          f"matrix task {t['id']} has no registered evaluator")
            self.assertNotIn("validate", t,
                             "matrix tasks must not carry inline validate snippets (§28)")

    def test_matrix_runner_parses_and_has_labels(self):
        import run_write_matrix as rwm
        self.assertTrue(hasattr(rwm, "run_variant"))
        import inspect
        sig = inspect.signature(rwm.run_variant)
        self.assertIn("agent_label", sig.parameters)
        self.assertIn("model_label", sig.parameters)

    def test_raw_receives_no_artifact(self):
        """§22: the raw variant must get NO context artifact — the cell
        builder may only generate artifacts for scc-full/external
        variants."""
        import run_write_matrix as rwm
        src = inspect.getsource(rwm.run_variant)
        self.assertIn('variant == "scc-full"', src)
        self.assertIn("EXTERNAL_VARIANTS", src)
        # raw falls through with artifact=None
        self.assertIn("artifact = None", src)

    def test_metadata_records_labels_not_hardcoded_agent(self):
        import run_write_matrix as rwm
        src = inspect.getsource(rwm.main)
        self.assertIn("--agent-label", src)
        self.assertIn("--model-label", src)
        self.assertNotIn('"agent": "codex"', src,
                         "agent label must come from --agent-label, never hardcoded")


import inspect  # noqa: E402  (used by the wiring tests above)


class TestFullSCCContainsStructural(unittest.TestCase):
    """§18: 'Do not call something SCC Full if Structural Source is
    missing.' The authoritative builder must emit the structural section."""

    def test_scc_full_artifact_has_structural_source(self):
        import evaluators as ev
        import subprocess, tempfile, shutil, json, os
        scc_bin = os.environ.get("SCC_BIN") or shutil.which("scc")
        if not scc_bin:
            self.skipTest("scc binary not available")
        with tempfile.TemporaryDirectory(prefix="scc-full-struct-") as td:
            proc = subprocess.run(
                [scc_bin, "bench", "external", "--variant", "scc-full",
                 "--repo", "http-service-python", "--budget", "4000",
                 "--artifact-only", "add a health check endpoint",
                 "--workdir", td],
                capture_output=True, text=True, timeout=300)
            self.assertEqual(proc.returncode, 0, proc.stderr[-400:])
            payload = json.loads(proc.stdout)
            text = Path(payload["artifact"]).read_text()
            self.assertIn("STRUCTURAL SOURCE", text,
                          "Full SCC artifact missing the Structural Source section")
            self.assertIn("SYSTEM ATLAS", text,
                          "Full SCC artifact missing the Atlas section")
            self.assertIn("SURFACE", text.upper(),
                          "Full SCC artifact missing the Surface section")
            # §19: equal-token postcondition on the FINAL artifact.
            actual = max(1, len(text) // 4)
            self.assertLessEqual(actual, 4000,
                                  f"final artifact {actual} tokens > budget 4000")


class TestPromptEvaluatorAlignment(unittest.TestCase):
    """§27: the task prompt must state the observable success criteria the
    evaluator enforces — no hidden criteria, no leakage of target
    files/symbols/solution."""

    def test_goals_state_the_evaluator_observable(self):
        import evaluators as ev
        # Each entry: words the GOAL must contain because the evaluator
        # enforces exactly that observable behavior.
        required = {
            "http-service.rename-transcript-field": ["transcriptText"],
            "http-service.transcript-normalization": ["raw text"],
            "http-service.health-check": ["readiness", "status"],
            "py-queue.empty-messages": ["empty"],
            "py-queue.classification-fallback": ["fallback"],
            "queue-worker.asr-retry": ["retry"],
            "queue-worker.street-vocabulary": ["vocabulary"],
            "ts-api-web.pagination": ["pagination"],
        }
        goals = {t["id"]: t["goal"] for t in ev.load_tasks()}
        for task_id, words in required.items():
            with self.subTest(task=task_id):
                self.assertIn(task_id, goals)
                for w in words:
                    self.assertIn(w, goals[task_id],
                                  f"goal for {task_id} must state the observable '{w}'")

    def test_goals_do_not_leak_targets(self):
        """§27: no target file, target symbol, or solution in the goal."""
        import evaluators as ev
        leaks = {
            "http-service.rename-transcript-field": ["transcripts.py", "find_all", "dict(r)"],
            "http-service.health-check": ["main.py", "def health"],
            "py-queue.empty-messages": ["consumer.py", "message.get"],
            "queue-worker.asr-retry": ["ingest.ts", "for (let attempt"],
            "ts-api-web.pagination": ["users.ts", "findMany", "take"],
        }
        goals = {t["id"]: t["goal"] for t in ev.load_tasks()}
        for task_id, banned in leaks.items():
            with self.subTest(task=task_id):
                for b in banned:
                    self.assertNotIn(b, goals.get(task_id, ""),
                                     f"goal for {task_id} leaks target {b!r}")
