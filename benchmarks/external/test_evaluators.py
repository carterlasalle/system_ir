"""Behavioral evaluator contract: comment-only patches MUST fail.

These tests would have caught the P0 where six grep validators treated
``# transcriptText`` / ``// retry`` comments as successful implementations.
"""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "fixtures"
EVALUATOR = ROOT / "benchmarks" / "evaluators" / "run.py"
TASKS_JSON = ROOT / "benchmarks" / "tasks.json"

SCOPED = [
    ("http-service.rename-transcript-field", "http-service-python", "# transcriptText"),
    ("queue-worker.asr-retry", "queue-worker-ts", "// retry"),
    ("ts-api-web.pagination", "ts-api-web", "// page"),
    ("py-queue.empty-messages", "py-queue-service", "# if not message:"),
    ("http-service.health-check", "http-service-python", "# uptime"),
    ("queue-worker.street-vocabulary", "queue-worker-ts",
     "// Map<string, string> includes replace street vocabulary"),
]


def _copy_fixture(src: Path) -> Path:
    tmp = Path(tempfile.mkdtemp(prefix="scc-eval-"))
    dest = tmp / src.name
    shutil.copytree(src, dest)
    return dest


def _comment_only(repo: Path, comment: str) -> None:
    for path in repo.rglob("*"):
        if not path.is_file():
            continue
        if path.suffix not in {".py", ".ts", ".js", ".tsx"}:
            continue
        path.write_text(path.read_text(encoding="utf-8") + "\n" + comment + "\n", encoding="utf-8")
        return
    raise AssertionError(f"no source file to comment in {repo}")


def run_eval(task_id: str, repo: Path) -> tuple[bool, str]:
    proc = subprocess.run(
        [sys.executable, str(EVALUATOR), task_id],
        cwd=repo,
        capture_output=True,
        text=True,
        timeout=45,
    )
    return proc.returncode == 0, (proc.stdout or "") + (proc.stderr or "")


class BehavioralEvaluatorsTest(unittest.TestCase):
    def test_untouched_fixtures_fail_all_six_scoped(self) -> None:
        for task_id, repo_name, _comment in SCOPED:
            with self.subTest(task=task_id):
                repo = _copy_fixture(FIXTURES / repo_name)
                try:
                    ok, detail = run_eval(task_id, repo)
                    self.assertFalse(
                        ok,
                        f"{task_id} passed on untouched fixture: {detail}",
                    )
                finally:
                    shutil.rmtree(repo.parent, ignore_errors=True)

    def test_comment_only_patches_fail_all_six(self) -> None:
        """Red gate: a comment-only patch MUST NOT be treated as success."""
        failures = []
        for task_id, repo_name, comment in SCOPED:
            repo = _copy_fixture(FIXTURES / repo_name)
            try:
                _comment_only(repo, comment)
                ok, detail = run_eval(task_id, repo)
                if ok:
                    failures.append(f"{task_id}: comment-only passed ({detail.strip()})")
            finally:
                shutil.rmtree(repo.parent, ignore_errors=True)
        self.assertEqual(
            failures,
            [],
            "comment-only patches must fail all six behavioral evaluators; "
            + "; ".join(failures),
        )

    def test_real_health_implementation_passes(self) -> None:
        repo = _copy_fixture(FIXTURES / "http-service-python")
        try:
            target = repo / "main.py"
            text = target.read_text(encoding="utf-8")
            text = text.replace(
                'return {"status": "ok"}',
                'return {"status": "ok", "uptime": 1.5}',
            )
            target.write_text(text, encoding="utf-8")
            ok, detail = run_eval("http-service.health-check", repo)
            self.assertTrue(ok, detail)
        finally:
            shutil.rmtree(repo.parent, ignore_errors=True)

    def test_every_canonical_task_has_evaluator(self) -> None:
        tasks = json.loads(TASKS_JSON.read_text(encoding="utf-8"))["tasks"]
        src = EVALUATOR.read_text(encoding="utf-8")
        missing = []
        for t in tasks:
            tid = t["id"]
            if f'"{tid}"' not in src:
                missing.append(tid)
            self.assertIn("validate", t, tid)
            self.assertIn("run.py", t["validate"])
        self.assertEqual(missing, [], f"missing DISPATCH entries: {missing}")

    def test_untouched_full_corpus_fails(self) -> None:
        """None of the 21 coding evaluators may pass the stock fixture."""
        tasks = json.loads(TASKS_JSON.read_text(encoding="utf-8"))["tasks"]
        failures = []
        for t in tasks:
            repo = _copy_fixture(FIXTURES / t["repo"])
            try:
                ok, detail = run_eval(t["id"], repo)
                if ok:
                    failures.append(f"{t['id']}: {detail.strip()}")
            finally:
                shutil.rmtree(repo.parent, ignore_errors=True)
        self.assertEqual(failures, [], "untouched fixtures must fail: " + "; ".join(failures))


if __name__ == "__main__":
    unittest.main()
