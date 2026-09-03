#!/usr/bin/env python3
"""Traced red-gate for behavioral evaluators (comment-only must fail).

Kept next to run.py so TraceLayer can see it (`benchmarks/external/**` is
excluded from the policy). CI also runs the fuller suite in
`benchmarks/external/test_evaluators.py`.
"""
from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "fixtures"
EVALUATOR = Path(__file__).resolve().parent / "run.py"

SCOPED = [
    ("http-service.rename-transcript-field", "http-service-python", "# transcriptText"),
    ("queue-worker.asr-retry", "queue-worker-ts", "// retry"),
    ("ts-api-web.pagination", "ts-api-web", "// page"),
    ("py-queue.empty-messages", "py-queue-service", "# if not message:"),
    ("http-service.health-check", "http-service-python", "# uptime"),
    ("queue-worker.street-vocabulary", "queue-worker-ts",
     "// Map<string, string> includes replace street vocabulary"),
]


def _copy(src: Path) -> Path:
    tmp = Path(tempfile.mkdtemp(prefix="scc-eval-contract-"))
    dest = tmp / src.name
    shutil.copytree(src, dest)
    return dest


def _eval(task_id: str, repo: Path) -> tuple[bool, str]:
    proc = subprocess.run(
        [sys.executable, str(EVALUATOR), task_id],
        cwd=repo,
        capture_output=True,
        text=True,
        timeout=45,
    )
    return proc.returncode == 0, (proc.stdout or "") + (proc.stderr or "")


class EvaluatorContractTest(unittest.TestCase):
    # trace:v1 id=test.scc.evaluators.comment-only-red-gate work=WORK-p0-omp-integration-correctness-and-writable-benchmark-scientific-validit verifies=REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient exercises=impl.scc.evaluators.evaluate-task
    def test_comment_only_patches_fail_all_six(self) -> None:
        failures = []
        for task_id, repo_name, comment in SCOPED:
            repo = _copy(FIXTURES / repo_name)
            try:
                written = False
                for path in repo.rglob("*"):
                    if path.is_file() and path.suffix in {".py", ".ts", ".js", ".tsx"}:
                        path.write_text(
                            path.read_text(encoding="utf-8") + "\n" + comment + "\n",
                            encoding="utf-8",
                        )
                        written = True
                        break
                self.assertTrue(written, task_id)
                ok, detail = _eval(task_id, repo)
                if ok:
                    failures.append(f"{task_id}: {detail.strip()}")
            finally:
                shutil.rmtree(repo.parent, ignore_errors=True)
        self.assertEqual(failures, [], "comment-only must fail: " + "; ".join(failures))

    # trace:v1 id=test.scc.evaluators.untouched-fail work=WORK-p0-omp-integration-correctness-and-writable-benchmark-scientific-validit verifies=REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient exercises=impl.scc.evaluators.evaluate-task
    def test_untouched_scoped_fixtures_fail(self) -> None:
        for task_id, repo_name, _ in SCOPED:
            with self.subTest(task=task_id):
                repo = _copy(FIXTURES / repo_name)
                try:
                    ok, detail = _eval(task_id, repo)
                    self.assertFalse(ok, f"{task_id} passed untouched: {detail}")
                finally:
                    shutil.rmtree(repo.parent, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
