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

    # trace:v1 id=test.scc.evaluators.nextjs-unused-transcript-text work=WORK-p0-omp-integration-correctness-and-writable-benchmark-scientific-validit verifies=REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient exercises=impl.scc.evaluators.evaluate-task
    def test_nextjs_unused_transcript_text_const_fails(self) -> None:
        repo = _copy(FIXTURES / "nextjs-fullstack")
        try:
            route = repo / "app" / "api" / "transcripts" / "route.ts"
            text = route.read_text(encoding="utf-8")
            before = text
            text = text.replace("{ transcript: record }", "{ wrongField: record }")
            self.assertNotEqual(text, before, "response field replacement did not apply")
            before = text
            text = text.replace(
                "export async function GET(req: Request) {",
                "export async function GET(req: Request) {\n  const transcriptText = 'unused';",
            )
            self.assertNotEqual(text, before, "unused const insertion did not apply")
            route.write_text(text, encoding="utf-8")
            ok, detail = _eval("nextjs.transcript-response", repo)
            self.assertFalse(ok, "unused transcriptText must fail: " + detail)
        finally:
            shutil.rmtree(repo.parent, ignore_errors=True)

    def test_creation_test_invocation_without_assertion_fails(self) -> None:
        repo = _copy(FIXTURES / "ts-api-web")
        try:
            (repo / "web" / "view.test.ts").write_text(
                'describe("create", () => { it("calls", async () => { await createUser({ name: "Ada" }); }); });\n',
                encoding="utf-8",
            )
            ok, detail = _eval("ts-api-web.creation-test", repo)
            self.assertFalse(ok, "createUser without expect must fail: " + detail)
        finally:
            shutil.rmtree(repo.parent, ignore_errors=True)

    def test_new_field_rejects_transcript_rename_only(self) -> None:
        repo = _copy(FIXTURES / "monorepo-acceptance")
        try:
            routes = repo / "api" / "routes.ts"
            before = routes.read_text(encoding="utf-8")
            text = before.replace("transcript: record.raw_text", "transcriptText: record.raw_text")
            self.assertNotEqual(text, before, "transcript rename replacement did not apply")
            routes.write_text(text, encoding="utf-8")
            self.assertIn("transcriptText", routes.read_text(encoding="utf-8"))
            ok, detail = _eval("monorepo.new-field", repo)
            self.assertFalse(ok, "rename-only must fail new-field: " + detail)
        finally:
            shutil.rmtree(repo.parent, ignore_errors=True)

    def test_currency_requires_amount_and_currency(self) -> None:
        repo = _copy(FIXTURES / "polyglot-monorepo")
        try:
            app = repo / "web" / "app.ts"
            text = app.read_text(encoding="utf-8")
            before = text
            text = text.replace(
                "return res.data.map((p: { amount: number }) => `$${p.amount}`).join(\", \");",
                'return "currency";',
            )
            self.assertNotEqual(text, before)
            app.write_text(text, encoding="utf-8")
            ok, detail = _eval("polyglot.web-payments", repo)
            self.assertFalse(ok, "currency word without amount must fail: " + detail)
        finally:
            shutil.rmtree(repo.parent, ignore_errors=True)

    def test_refund_unreachable_call_does_not_pass(self) -> None:
        repo = _copy(FIXTURES / "polyglot-monorepo")
        try:
            pay = repo / "svc" / "payments.py"
            pay.write_text(
                pay.read_text(encoding="utf-8") + "\ndef handle_refund(pid=None):\n    return {}\n",
                encoding="utf-8",
            )
            server = repo / "svc" / "server.py"
            server.write_text(
                server.read_text(encoding="utf-8") + "\nif False:\n    handle_refund('never')\n",
                encoding="utf-8",
            )
            ok, detail = _eval("polyglot.refund-endpoint", repo)
            self.assertFalse(ok, "if False handle_refund must not pass: " + detail)
        finally:
            shutil.rmtree(repo.parent, ignore_errors=True)

    def test_pagination_constant_two_items_fails(self) -> None:
        repo = _copy(FIXTURES / "ts-api-web")
        try:
            users = repo / "service" / "users.ts"
            text = users.read_text(encoding="utf-8")
            before = text
            text = text.replace(
                "  const rows = await db.users.findMany({ select: { id: true, name: true } });\n"
                "  return rows;",
                "  return [{ id: '1', name: 'a' }, { id: '2', name: 'b' }];",
            )
            self.assertNotEqual(text, before)
            users.write_text(text, encoding="utf-8")
            ok, detail = _eval("ts-api-web.pagination", repo)
            self.assertFalse(ok, "always-two-items list must fail: " + detail)
        finally:
            shutil.rmtree(repo.parent, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
