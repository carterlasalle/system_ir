"""Writable-protocol contract tests (P0 experimental rigor).

Catches: Python scc-full missing Structural Source, indexing the original
fixture tree instead of the cell copy, agent label hardcoded to Codex,
and concatenation that ignores the declared token budget.
"""
from __future__ import annotations

import json
import os
import shutil
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
    def test_estimate_shared_tokens_is_quarter_bytes(self) -> None:
        self.assertEqual(estimate_shared_tokens("abcd"), 1)
        self.assertEqual(estimate_shared_tokens("a" * 40), 10)

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
    def test_agent_label_follows_agent_cmd_not_hardcoded_codex(self) -> None:
        src = (EXT / "run_write_matrix.py").read_text(encoding="utf-8")
        self.assertNotIn('"agent": "codex"', src)
        self.assertIn("infer_agent_name", src)
        self.assertIn("scc_revision", src)
        self.assertIn("harness_revision", src)
        self.assertIn("agent_cmd", src)
        self.assertIn("micro_task_success", src)
        self.assertIn("paired_ci_scc_minus_raw", src)
        self.assertIn("paired_ci_scc_minus_aider", src)
        self.assertIn("paired_ci_scc_minus_repomix", src)

        from run_write_matrix import infer_agent_name

        self.assertEqual(infer_agent_name("claude --print"), "claude")
        self.assertEqual(infer_agent_name("/usr/bin/codex exec"), "codex")
        self.assertEqual(infer_agent_name("/opt/claude-code/claude"), "claude")
        self.assertEqual(infer_agent_name("aider --yes"), "aider")

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
        self.assertIn("h.run_writable_variant", matrix)
        # A split native/external runner was the P0: native must not take
        # a private copy+run_write_task path while aider uses the other.
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


if __name__ == "__main__":
    unittest.main()
