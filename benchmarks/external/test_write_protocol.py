"""Writable-protocol contract tests (P0 experimental rigor).

Catches: Python scc-full missing Structural Source, indexing the original
fixture tree instead of the cell copy, agent label hardcoded to Codex,
and concatenation that ignores the declared token budget.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
EXT = Path(__file__).resolve().parent
sys.path.insert(0, str(EXT))

from run_context_bench import (  # noqa: E402
    WRITABLE_CODING_VARIANTS,
    aggregate,
    build_scc_variant_artifact,
    estimate_shared_tokens,
)


def _scc_bin() -> str | None:
    env = os.environ.get("SCC_BIN")
    if env and Path(env).exists():
        return env
    local = ROOT / "target" / "debug" / "scc"
    if local.exists():
        return str(local)
    return shutil.which("scc")


class TokenBudgetTest(unittest.TestCase):
    def test_estimate_shared_tokens_is_chars_ceiling(self) -> None:
        # Mirror of Rust scc_core::estimate_tokens (chars().count().div_ceil(4)).
        self.assertEqual(estimate_shared_tokens(""), 0)
        self.assertEqual(estimate_shared_tokens("abcd"), 1)
        self.assertEqual(estimate_shared_tokens("abcde"), 2)
        self.assertEqual(estimate_shared_tokens("a" * 40), 10)

    def test_estimate_shared_tokens_covers_content_kinds(self) -> None:
        # ASCII code, Unicode (code points, like Rust chars), identifier-heavy
        # code, JSON, Markdown, and long file paths all estimate positively
        # and never exceed a chars/4 ceiling.
        samples = [
            "fn main() { println!(\"hi\"); }\n",
            "日本語テスト✓🎉\n",
            "very_long_identifier_name_xyz.viewDidLoadTableViewCellForRowAt();\n",
            '{"route": "/api/v1/transcripts", "method": "GET", "auth": true}\n',
            "# Title\n\n- item one\n- item two\n\n`code span`\n",
            "crates/scc-context/src/surface/structural_source_selection_policy.rs\n",
        ]
        for s in samples:
            est = estimate_shared_tokens(s)
            self.assertGreater(est, 0, repr(s))
            self.assertLessEqual(est, (len(s) + 3) // 4, repr(s))
            self.assertEqual(est, (len(s) + 3) // 4, repr(s))

    def test_concatenation_is_capped_at_budget(self) -> None:
        """A large concat must not ship an over-budget prompt."""
        src = (EXT / "run_context_bench.py").read_text(encoding="utf-8")
        self.assertIn("text[: budget * 4]", src)
        budget = 50
        huge = "a" * 4000
        self.assertGreater(estimate_shared_tokens(huge), budget)
        capped = huge[: budget * 4]
        self.assertLessEqual(estimate_shared_tokens(capped), budget)


class SccFullStructuralTest(unittest.TestCase):
    def test_scc_full_includes_structural_source_and_indexes_copy(self) -> None:
        scc = _scc_bin()
        if not scc:
            self.skipTest("scc binary not built")
        src_fixture = ROOT / "fixtures" / "http-service-python"
        tmp = Path(tempfile.mkdtemp(prefix="scc-full-"))
        try:
            copy = tmp / "repo"
            shutil.copytree(src_fixture, copy)
            cmds: list[list[str]] = []
            import run_context_bench as rcb

            real_run = rcb.subprocess.run

            def spy(*args, **kwargs):
                cmd = args[0] if args else kwargs.get("args")
                if isinstance(cmd, (list, tuple)):
                    cmds.append(list(cmd))
                return real_run(*args, **kwargs)

            with mock.patch.object(rcb.subprocess, "run", side_effect=spy):
                text, tokens, err = build_scc_variant_artifact(
                    "scc-full",
                    copy,
                    "Add uptime to /health",
                    tmp,
                    scc,
                    8000,
                )
            self.assertIsNone(err, err)
            self.assertIsNotNone(text)
            index_cmds = [c for c in cmds if len(c) > 1 and "index" in c]
            self.assertTrue(index_cmds, f"expected scc index, got {cmds}")
            for c in index_cmds:
                self.assertIn("--root", c)
                root = c[c.index("--root") + 1]
                self.assertEqual(Path(root).resolve(), copy.resolve())
                self.assertNotEqual(Path(root).resolve(), src_fixture.resolve())
            structural = [
                c for c in cmds
                if len(c) > 2 and "context" in c and "structural" in c
            ]
            self.assertTrue(
                structural,
                "scc-full must call `scc context structural` (Structural Source)",
            )
            artifact = Path(text).read_text(encoding="utf-8") if text else ""
            self.assertIn("source:", artifact)
            self.assertLessEqual(tokens, 8000)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)

    def test_scc_full_is_not_startup_plus_task_only(self) -> None:
        """The old Python path concatenated startup+pack+delta and called it full."""
        src = (EXT / "run_context_bench.py").read_text(encoding="utf-8")
        self.assertIn('["context", "structural", "--task"', src)
        self.assertIn("structural_budget", src)


class WriteMatrixMetadataTest(unittest.TestCase):
    def test_agent_label_is_explicit_not_hardcoded_codex(self) -> None:
        src = (EXT / "run_write_matrix.py").read_text(encoding="utf-8")
        self.assertNotIn('"agent": "codex"', src)
        self.assertIn("--agent-label", src)
        self.assertIn("agent_cmd", src)
        self.assertIn("scc_commit", src)
        self.assertIn("rev-parse", src)
        self.assertNotIn('scc_bin, "--version"', src)
        self.assertIn("compute_summary", src)
        self.assertIn("paired_", src)

    def test_writable_flag_routes_native_through_writable_runner(self) -> None:
        src = (EXT / "run_context_bench.py").read_text(encoding="utf-8")
        self.assertIn("run_writable_variant", src)
        self.assertIn("writable=args.writable", src)
        self.assertIn("WRITABLE_CODING_VARIANTS", src)
        self.assertIn("if args.writable and args.variant is None:", src)
        self.assertEqual(
            set(WRITABLE_CODING_VARIANTS),
            {
                "raw",
                "aider-repomap",
                "repomix-compress",
                "scc-atlas",
                "scc-surface",
                "scc-atlas-surface",
                "scc-full",
            },
        )
        matrix = (EXT / "run_write_matrix.py").read_text(encoding="utf-8")
        self.assertIn("scc_artifact", matrix)
        self.assertIn("run_write_task", matrix)
        self.assertNotIn("h.build_scc_variant_artifact", matrix)


class AggregateNamingTest(unittest.TestCase):
    def test_aggregate_emits_micro_and_macro(self) -> None:
        rows = [
            {
                "variant": "scc-full",
                "repo": "a",
                "budget": 8000,
                "tasks": 2,
                "tasks_passed": 1,
                "tasks_with_evaluator": 2,
                "task_success_rate": 0.5,
                "run_completion_rate": 1.0,
                "context_tokens": 10,
                "budget": 8000,
            },
            {
                "variant": "scc-full",
                "repo": "b",
                "budget": 8000,
                "tasks": 1,
                "tasks_passed": 1,
                "tasks_with_evaluator": 1,
                "task_success_rate": 1.0,
                "run_completion_rate": 1.0,
                "context_tokens": 10,
            },
        ]
        summary = aggregate(rows)
        self.assertIn("micro_task_success", summary)
        self.assertIn("macro_repo_success", summary)
        self.assertAlmostEqual(summary["micro_task_success"], 2 / 3)
        self.assertAlmostEqual(summary["macro_repo_success"], 0.75)

    def test_aggregate_propagates_infra_failures(self) -> None:
        rows = [
            {
                "variant": "scc-full",
                "repo": "a",
                "budget": 8000,
                "tasks": 1,
                "tasks_passed": 0,
                "tasks_with_evaluator": 0,
                "task_success_rate": None,
                "run_completion_rate": 0.0,
                "context_tokens": 0,
                "infra_failed": 1,
                "error": "ARTIFACT_FAILED: timeout",
            }
        ]
        summary = aggregate(rows)
        self.assertEqual(summary["infra_failed"], 1)
        self.assertIn("ARTIFACT_FAILED", summary["error"])
        self.assertFalse(summary.get("valid", True))


class InvalidResultsMarkedTest(unittest.TestCase):
    def test_legacy_write_matrix_files_are_marked_invalid(self) -> None:
        results = ROOT / "benchmarks" / "results"
        for name in (
            "write-matrix.json",
            "write-matrix-v2.json",
            "write-matrix-full.json",
            "write-matrix-claude.json",
        ):
            path = results / name
            self.assertTrue(path.exists(), name)
            payload = json.loads(path.read_text(encoding="utf-8"))
            meta = payload.get("meta") or payload
            self.assertFalse(
                meta.get("valid", True),
                f"{name} must set valid: false",
            )
            self.assertTrue(meta.get("invalid_reason"), f"{name} needs invalid_reason")


class NativeDefaultBudgetTest(unittest.TestCase):
    def test_native_default_omits_budget_flag_on_scc_argv(self) -> None:
        scc = "scc"
        src_fixture = ROOT / "fixtures" / "http-service-python"
        tmp = Path(tempfile.mkdtemp(prefix="scc-native-budget-"))
        try:
            copy = tmp / "repo"
            shutil.copytree(src_fixture, copy)
            cmds: list[list[str]] = []
            import run_context_bench as rcb

            def spy(*args, **kwargs):
                cmd = args[0] if args else kwargs.get("args")
                if isinstance(cmd, (list, tuple)):
                    cmds.append(list(cmd))
                return mock.Mock(returncode=0, stdout="source: hello\n" * 40, stderr="")

            with mock.patch.object(rcb.subprocess, "run", side_effect=spy):
                text, _tokens, err = build_scc_variant_artifact(
                    "scc-full", copy, "goal", tmp, scc, None,
                )
            self.assertIsNone(err, err)
            self.assertIsNotNone(text)
            contextish = [
                c for c in cmds
                if any(tok in c for tok in ("atlas", "surface", "context", "startup", "structural"))
            ]
            self.assertTrue(contextish, cmds)
            for c in contextish:
                self.assertNotIn("--budget", c, f"native-default must omit --budget: {c}")
        finally:
            shutil.rmtree(tmp, ignore_errors=True)

    def test_equal_token_passes_budget_flag(self) -> None:
        src_fixture = ROOT / "fixtures" / "http-service-python"
        tmp = Path(tempfile.mkdtemp(prefix="scc-eq-budget-"))
        try:
            copy = tmp / "repo"
            shutil.copytree(src_fixture, copy)
            cmds: list[list[str]] = []
            import run_context_bench as rcb

            def spy(*args, **kwargs):
                cmd = args[0] if args else kwargs.get("args")
                if isinstance(cmd, (list, tuple)):
                    cmds.append(list(cmd))
                return mock.Mock(returncode=0, stdout="source: hello\n" * 40, stderr="")

            with mock.patch.object(rcb.subprocess, "run", side_effect=spy):
                build_scc_variant_artifact("scc-full", copy, "goal", tmp, "scc", 8000)
            contextish = [c for c in cmds if "context" in c]
            self.assertTrue(any("--budget" in c for c in contextish), cmds)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


class WritableCellIsolationTest(unittest.TestCase):
    def test_cells_are_keyed_by_budget_and_recreated(self) -> None:
        import run_context_bench as rcb

        tmp = Path(tempfile.mkdtemp(prefix="scc-cell-"))
        orig_write = rcb.run_write_task
        seen: list[Path] = []
        files_at_start: list[set[str]] = []

        def fake_write(agent_cmd, root, goal, **kwargs):
            root = Path(root)
            seen.append(root)
            files_at_start.append({p.name for p in root.rglob("*") if p.is_file()})
            (root / "AGENT_CREATED.py").write_text("stale", encoding="utf-8")
            return {
                "run_completion": True,
                "task_success": True,
                "task_success_defined": True,
                "patch_produced": True,
                "wall_sec": 0.1,
            }

        tasks = {
            "http-service-python": [
                {"id": "http-service.health-check", "goal": "g", "validate": "true"}
            ]
        }
        try:
            rcb.run_write_task = fake_write
            rcb.run_writable_variant("raw", tasks, 8000, "true", tmp)
            rcb.run_writable_variant("raw", tasks, 2000, "true", tmp)
            self.assertEqual(len(seen), 2)
            self.assertNotEqual(seen[0], seen[1])
            self.assertTrue(any("8000" in p.parts for p in seen), seen)
            self.assertTrue(any("2000" in p.parts for p in seen), seen)
            self.assertTrue((seen[0] / "AGENT_CREATED.py").exists())
            rcb.run_writable_variant("raw", tasks, 8000, "true", tmp)
            self.assertEqual(len(seen), 3)
            self.assertNotIn(
                "AGENT_CREATED.py",
                files_at_start[2],
                "rmtree must drop the previous cell before copy_tree",
            )
            self.assertIn("8000", seen[2].parts)
        finally:
            rcb.run_write_task = orig_write
            shutil.rmtree(tmp, ignore_errors=True)


class ArtifactFailureNotTaskOutcomeTest(unittest.TestCase):
    def test_artifact_error_is_not_counted_as_evaluator_failure(self) -> None:
        import run_context_bench as rcb

        tmp = Path(tempfile.mkdtemp(prefix="scc-artfail-"))
        orig = rcb.build_scc_variant_artifact
        try:
            rcb.build_scc_variant_artifact = lambda *a, **k: (None, 0, "scc index failed")
            rows, skipped = rcb.run_writable_variant(
                "scc-full",
                {"http-service-python": [{"id": "t1", "goal": "g", "validate": "true"}]},
                8000,
                "true",
                tmp,
                scc_bin="scc",
            )
            self.assertIsNone(skipped)
            self.assertEqual(rows[0]["tasks_with_evaluator"], 0)
            self.assertIsNone(rows[0]["task_success_rate"])
            self.assertTrue(rows[0]["error"])
            self.assertGreaterEqual(rows[0]["infra_failed"], 1)
        finally:
            rcb.build_scc_variant_artifact = orig
            shutil.rmtree(tmp, ignore_errors=True)

    def test_artifact_timeout_is_recorded_without_aborting_the_matrix(self) -> None:
        import subprocess
        import run_context_bench as rcb

        tmp = Path(tempfile.mkdtemp(prefix="scc-arttimeout-"))
        orig = rcb.build_scc_variant_artifact
        try:
            def boom(*_a, **_k):
                raise subprocess.TimeoutExpired(cmd="scc", timeout=1)

            rcb.build_scc_variant_artifact = boom
            rows, skipped = rcb.run_writable_variant(
                "scc-full",
                {"http-service-python": [{"id": "t1", "goal": "g", "validate": "true"}]},
                8000,
                "true",
                tmp,
                scc_bin="scc",
            )
            self.assertIsNone(skipped)
            self.assertEqual(len(rows), 1)
            self.assertGreaterEqual(rows[0]["infra_failed"], 1)
            self.assertIsNone(rows[0]["task_success_rate"])
            self.assertIn("TimeoutExpired", rows[0]["error"] or "")
        finally:
            rcb.build_scc_variant_artifact = orig
            shutil.rmtree(tmp, ignore_errors=True)


class WriteMatrixPairingAndValidityTest(unittest.TestCase):
    def _load_matrix(self):
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "run_write_matrix", EXT / "run_write_matrix.py"
        )
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    def test_paired_ci_excludes_mismatched_and_undefined_outcomes(self) -> None:
        rwm = self._load_matrix()
        cells = {
            "scc-full/a": {"task_success": True},
            "scc-full/b": {"task_success": False, "error": "artifact"},
            "raw/a": {"task_success": False, "error": "artifact"},
            "raw/b": {"task_success": True},
        }
        summary = rwm.compute_summary(cells, ["a", "b"], ["raw", "scc-full"])
        self.assertIsNone(summary["paired_scc-full_minus_raw"])
        self.assertIn("b", summary["skipped_cells"]["scc-full"])
        self.assertIn("a", summary["skipped_cells"]["raw"])

        cells["raw/a"] = {"task_success": False}
        summary = rwm.compute_summary(cells, ["a", "b"], ["raw", "scc-full"])
        self.assertEqual(summary["paired_scc-full_minus_raw"]["n_paired"], 1)

        cells["scc-full/c"] = {"task_success": None, "status": "SKIPPED"}
        cells["raw/c"] = {"task_success": True}
        summary = rwm.compute_summary(cells, ["a", "b", "c"], ["raw", "scc-full"])
        self.assertEqual(summary["paired_scc-full_minus_raw"]["n_paired"], 1)
        self.assertEqual(summary["skipped_cells"]["scc-full"]["c"], "SKIPPED")
        self.assertEqual(summary["scc-full_n"], 1)

    def test_skipped_cell_without_error_text_is_excluded(self) -> None:
        rwm = self._load_matrix()
        cells = {
            "raw/a": {"task_success": True},
            "raw/b": {"task_success": None},
            "scc-full/a": {"task_success": True},
            "scc-full/b": {"task_success": None},
        }
        summary = rwm.compute_summary(cells, ["a", "b"], ["raw", "scc-full"])
        self.assertEqual(summary["raw_n"], 1)
        self.assertEqual(summary["scc-full_n"], 1)
        self.assertAlmostEqual(summary["raw_task_success"], 1.0)
        self.assertIn("b", summary["skipped_cells"]["raw"])
        self.assertEqual(summary["skipped_cells"]["raw"]["b"], "undefined-outcome")

    def test_incomplete_matrix_sets_valid_false(self) -> None:
        rwm = self._load_matrix()
        out = Path(tempfile.mkdtemp(prefix="scc-matrix-")) / "out.json"

        def boom(*_a, **_k):
            raise RuntimeError("agent timeout")

        orig = rwm.run_variant
        try:
            rwm.run_variant = boom
            with self.assertRaises(RuntimeError):
                with mock.patch.dict(os.environ, {"SCC_ALLOW_PAID_BENCHMARKS": "1"}), \
                        mock.patch.object(rwm, "_run_preflight", return_value=0):
                    rwm.main([
                        "--out", str(out),
                        "--variants", "raw",
                        "--tasks", "http-service.health-check",
                    ])
            payload = json.loads(out.read_text(encoding="utf-8"))
            self.assertFalse(payload["meta"].get("valid", True))
            self.assertIn("incomplete", payload["meta"].get("invalid_reason", "").lower())
        finally:
            rwm.run_variant = orig
            shutil.rmtree(out.parent, ignore_errors=True)

    def test_infra_cell_marks_complete_matrix_invalid(self) -> None:
        rwm = self._load_matrix()
        out = Path(tempfile.mkdtemp(prefix="scc-matrix-infra-")) / "out.json"
        orig = rwm.run_variant
        try:
            rwm.run_variant = lambda *a, **k: {
                "task_success": None,
                "run_completion": False,
                "wall_sec": 0,
                "error": "scc-artifact-generation-failed",
                "status": "ARTIFACT_FAILED",
            }
            with mock.patch.dict(os.environ, {"SCC_ALLOW_PAID_BENCHMARKS": "1"}), \
                    mock.patch.object(rwm, "_run_preflight", return_value=0):
                rc = rwm.main([
                    "--out", str(out),
                    "--variants", "raw",
                    "--tasks", "http-service.health-check",
                ])
            self.assertEqual(rc, 0)
            payload = json.loads(out.read_text(encoding="utf-8"))
            self.assertFalse(payload["meta"].get("valid", True))
            self.assertIn("infrastructure", payload["meta"].get("invalid_reason", "").lower())
        finally:
            rwm.run_variant = orig
            shutil.rmtree(out.parent, ignore_errors=True)

    def test_scc_commit_is_git_head_not_package_version(self) -> None:
        src = (EXT / "run_write_matrix.py").read_text(encoding="utf-8")
        self.assertIn("scc_commit", src)
        self.assertIn('rev-parse", "HEAD"', src)
        self.assertNotIn("scc_revision", src)
        self.assertNotIn("[scc_bin, \"--version\"]", src)

class PaidBenchmarkGateTest(unittest.TestCase):
    """Paid-model safety interlock: normal invocation refuses before any
    external process/model launch; --dry-run is allowed; explicit
    SCC_ALLOW_PAID_BENCHMARKS=1 opts in."""

    def _load_matrix(self):
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "run_write_matrix", EXT / "run_write_matrix.py"
        )
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    def test_normal_invocation_refuses_before_launch(self) -> None:
        rwm = self._load_matrix()
        out = Path(tempfile.mkdtemp(prefix="scc-matrix-gate-")) / "out.json"
        try:
            def boom(*_a, **_k):
                raise AssertionError("must not reach agent launch without opt-in")

            orig = rwm.run_variant
            rwm.run_variant = boom
            try:
                with _drop_paid_env():
                    rc = rwm.main([
                        "--out", str(out),
                        "--variants", "raw",
                        "--tasks", "http-service.health-check",
                    ])
            finally:
                rwm.run_variant = orig
            self.assertEqual(rc, 2)
            self.assertFalse(out.exists(), "refused run must not write results")
        finally:
            shutil.rmtree(out.parent, ignore_errors=True)

    def test_dry_run_allowed_without_opt_in(self) -> None:
        rwm = self._load_matrix()
        out = Path(tempfile.mkdtemp(prefix="scc-matrix-dry-")) / "out.json"
        try:
            with _drop_paid_env():
                rc = rwm.main([
                    "--dry-run",
                    "--out", str(out),
                    "--variants", "raw",
                    "--tasks", "http-service.health-check",
                ])
            self.assertEqual(rc, 0)
            payload = json.loads(out.read_text(encoding="utf-8"))
            cell = payload["cells"]["raw/http-service.health-check"]
            self.assertEqual(cell["status"], "DRY-RUN")
            self.assertIsNone(cell["task_success"])
        finally:
            shutil.rmtree(out.parent, ignore_errors=True)

    def test_paid_run_consults_preflight(self) -> None:
        # opted-in but preflight refuses (no question): main must refuse
        rwm = self._load_matrix()
        out = Path(tempfile.mkdtemp(prefix="scc-matrix-preflight-")) / "out.json"
        try:
            with mock.patch.dict(os.environ, {"SCC_ALLOW_PAID_BENCHMARKS": "1"}), \
                    mock.patch.object(rwm, "_run_preflight", return_value=2) as pre:
                rc = rwm.main([
                    "--out", str(out),
                    "--variants", "raw",
                    "--tasks", "http-service.health-check",
                ])
            self.assertEqual(rc, 2)
            pre.assert_called_once()
            asked = pre.call_args[0][0]
            self.assertEqual(asked, "")
        finally:
            shutil.rmtree(out.parent, ignore_errors=True)

    def test_preflight_refuses_without_question(self) -> None:
        # preflight.py itself: no experimental question → REFUSE, no quota
        r = subprocess.run(
            [sys.executable, str(EXT / "preflight.py")],
            capture_output=True, text=True,
        )
        self.assertEqual(r.returncode, 2)
        self.assertIn("no experimental question", r.stdout)

    def test_explicit_opt_in_proceeds(self) -> None:
        rwm = self._load_matrix()
        out = Path(tempfile.mkdtemp(prefix="scc-matrix-optin-")) / "out.json"
        orig = rwm.run_variant
        try:
            rwm.run_variant = lambda *a, **k: {
                "task_success": True,
                "task_success_defined": True,
                "run_completion": True,
                "context_tokens": 10,
                "wall_sec": 0.1,
                "error": None,
                "error_type": None,
            }
            with mock.patch.dict(os.environ, {"SCC_ALLOW_PAID_BENCHMARKS": "1"}), \
                    mock.patch.object(rwm, "_run_preflight", return_value=0):
                rc = rwm.main([
                    "--out", str(out),
                    "--variants", "raw",
                    "--tasks", "http-service.health-check",
                ])
            self.assertEqual(rc, 0)
            payload = json.loads(out.read_text(encoding="utf-8"))
            self.assertTrue(payload["cells"]["raw/http-service.health-check"]["task_success"])
        finally:
            rwm.run_variant = orig
            shutil.rmtree(out.parent, ignore_errors=True)

    def test_dry_run_cells_excluded_from_summary(self) -> None:
        rwm = self._load_matrix()
        cells = {
            "raw/a": {"task_success": None, "status": "DRY-RUN"},
            "scc-full/a": {"task_success": None, "status": "DRY-RUN"},
        }
        summary = rwm.compute_summary(cells, ["a"], ["raw", "scc-full"])
        self.assertIsNone(summary["micro_task_success"])
        self.assertIsNone(summary["paired_scc-full_minus_raw"])


class _drop_paid_env:
    """Context manager removing the paid-benchmark opt-in for gate tests."""

    def __init__(self):
        self._had = os.environ.get("SCC_ALLOW_PAID_BENCHMARKS")

    def __enter__(self):
        os.environ.pop("SCC_ALLOW_PAID_BENCHMARKS", None)
        return self

    def __exit__(self, *exc):
        if self._had is not None:
            os.environ["SCC_ALLOW_PAID_BENCHMARKS"] = self._had
        return False


if __name__ == "__main__":
    unittest.main()
