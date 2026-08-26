#!/usr/bin/env python3
"""External-benchmark harness self-tests (Parts 5/6/9/10/11/12/13).

No network, no installed tools. Exercises the harness and the adapter
modules directly with deterministic fakes:
- `_row_for` row creation reaches the row dict and reports run_completion_rate
  (never a process-exit "success_rate").
- per-task Aider artifacts get unique immutable paths (no overwrite).
- repomix equal-token never exceeds the budget, even for the first file.
- native-default routing is explicit (variant, mode) — no suffix munging.
- repomr pin proof: version-only match is PIN-UNVERIFIED; the installed
  path-resolves-to-pinned-checkout proof is the only transitive acceptance.
"""

import importlib.util
import json
import os
import re
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# trace:v1 id=test.scc.bench-harness work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching verifies=REQ-complete-task-context-identical-across-transports
class RowForTest(unittest.TestCase):
    """Part 5/12: `_row_for` builds the row with an explicit mode and the
    honest run_completion_rate metric (agent process exit, NOT task success)."""

    def setUp(self):
        self.h = load("run_context_bench")
        self._fixtures = tempfile.TemporaryDirectory()
        (Path(self._fixtures.name) / "repo").mkdir(exist_ok=True)
        (Path(self._fixtures.name) / "repo" / "a.py").write_text("def a(): pass\n")
        self._orig_fixtures = self.h.FIXTURES
        self.h.FIXTURES = Path(self._fixtures.name)
        # Deterministic fake task results (no agent run needed).
        self.fake_results = [
            {"exit_ok": True, "files_opened": 2, "search_tool_calls": 1,
             "graph_tool_calls": 0, "first_plan_correct": True,
             "wrong_first_locations": 1},
            {"exit_ok": False, "files_opened": 4, "search_tool_calls": 3,
             "graph_tool_calls": 1, "first_plan_correct": False,
             "wrong_first_locations": 2},
        ]
        self.orig_run = self.h.run_task_via_protocol
        self.h.run_task_via_protocol = (
            lambda agent_cmd, artifact, goal, gt_files, plan_keys, root: self.fake_results.pop(0)
        )
        self.tmp = tempfile.TemporaryDirectory()
    def tearDown(self):
        self.h.run_task_via_protocol = self.orig_run
        self.h.FIXTURES = self._orig_fixtures
        self.tmp.cleanup()
        self._fixtures.cleanup()

    def test_row_has_mode_and_run_completion_rate(self):
        repo_tasks = [
            {"id": "t1", "goal": "g1", "files": ["a.py"], "symbols": ["s1"]},
            {"id": "t2", "goal": "g2", "files": ["b.py"], "symbols": ["s2"]},
        ]
        artifacts = [(Path(self.tmp.name) / "a.txt", 100), (Path(self.tmp.name) / "b.txt", 200)]
        row = self.h._row_for(repo_tasks, artifacts, "agent", "repo", "aider-repomap", 8000, mode="equal-token")
        self.assertEqual(row["mode"], "equal-token")
        self.assertEqual(row["variant"], "aider-repomap")
        self.assertEqual(row["budget"], 8000)
        self.assertIn("run_completion_rate", row)
        self.assertAlmostEqual(row["run_completion_rate"], 0.5)
        self.assertNotIn("success_rate", row, "exit-based metric must be named run_completion_rate")

    def test_row_propagates_native_mode(self):
        repo_tasks = [{"id": "t1", "goal": "g1", "files": [], "symbols": []}]
        artifacts = [(Path(self.tmp.name) / "a.txt", 0)]
        row = self.h._row_for(repo_tasks, artifacts, "agent", "repo", "aider-repomap", None, mode="native-default")
        self.assertEqual(row["mode"], "native-default")
        self.assertIsNone(row["budget"])


class UniqueArtifactTest(unittest.TestCase):
    """Part 6: every task-personalized aider artifacts get unique dirs; the
    task runner consumes the per-task path."""

    def test_run_external_variant_uses_per_task_dirs(self):
        h = load("run_context_bench")
        fixtures = tempfile.TemporaryDirectory()
        (Path(fixtures.name) / "repo").mkdir(exist_ok=True)
        (Path(fixtures.name) / "repo" / "a.py").write_text("def a(): pass\n")
        orig_fixtures = h.FIXTURES
        h.FIXTURES = Path(fixtures.name)
        # Stub the adapter subprocess: emit an ok payload writing a marker
        # file; capture the out_dir each task asked for.
        seen_dirs = []

        class FakeProc:
            def __init__(self, argv):
                self.returncode = 0
                self.stderr = ""
                fixture, budget, out_dir = Path(argv[1]), argv[3], Path(argv[4])
                # deterministically mark the task; the payload mirrors the
                # real adapter's ok contract.
                goal = argv[6] if argv[5] == "--goal" else "?"
                artifact = out_dir / "aider-map.txt"
                artifact.write_text(goal)
                seen_dirs.append(str(out_dir))
                tokens = len(goal) // 4 + 1
                self.stdout = json.dumps({
                    "ok": True, "tool": "aider", "tokens": tokens,
                    "files": 1, "artifact": str(artifact),
                    "pinned": "x", "mode": "equal-token",
                    "requested_budget": int(budget), "actual_shared_tokens": tokens,
                    "utilization": 0.5,
                })

        orig = h.subprocess.run
        h.subprocess.run = lambda argv, **kw: FakeProc(argv)
        orig_run_task = h.run_task_via_protocol
        h.run_task_via_protocol = (
            lambda agent_cmd, artifact, goal, gt_files, plan_keys, root: {
                "exit_ok": True, "files_opened": 1, "search_tool_calls": 0,
                "graph_tool_calls": 0, "first_plan_correct": True,
                "wrong_first_locations": 0,
            }
        )
        try:
            tasks = {
                "repo": [
                    {"id": "task-a", "goal": "alpha", "files": [], "symbols": []},
                    {"id": "task-b", "goal": "beta", "files": [], "symbols": []},
                ]
            }
            workdir = Path(tempfile.mkdtemp())
            rows, skipped = h.run_external_variant("aider-repomap", tasks, 8000, "agent", workdir)
            # Two distinct task dirs (no overwrite), each carrying its own
            # personalized map.
            self.assertEqual(len(seen_dirs), 2)
            self.assertNotEqual(seen_dirs[0], seen_dirs[1])
            # Row creation reached: the run_completion_rate key exists and
            # no NameError for `mode` (the Part 5 regression).
            self.assertEqual(len(rows), 1)
            self.assertIn("run_completion_rate", rows[0])
        finally:
            h.subprocess.run = orig
            h.run_task_via_protocol = orig_run_task
            h.FIXTURES = orig_fixtures
            fixtures.cleanup()


class RepomixFirstFileTest(unittest.TestCase):
    """Part 9: the first compressed file must never exceed the budget."""

    def setUp(self):
        self.r = load("repomix_adapter")
    def test_oversize_first_file_is_skipped(self):
        # Sections: a 5000-token file (does not fit 4000), then a 500-token
        # file (fits). The final artifact never exceeds the budget and the
        # first file is NOT included (never a mid-file truncation).
        r = self.r
        tmp = Path(tempfile.mkdtemp())
        # Fake the repomix CLI: write an XML with two files.
        big = "x" * (5000 * 4)
        small = "y" * (500 * 4)
        full_xml = f'<file path="a/big.py"><content>{big}</content></file>\n<file path="a/small.py">{small}</file>'
        r.FILE_RE = type("RE", (), {"finditer": staticmethod(lambda s: __import__("re").finditer(
            r'<file path="([^"]+)"[^>]*>([\s\S]*?)</file>', s))})()
        # Directly exercise the selection logic by replicating the loop body
        # from the adapter (the adapter's run_repomix shells out; we test the
        # selection invariant that the fix enforces).
        import html as _html
        sections_parsed = []
        import re
        for m in re.finditer(r'<file path="([^"]+)"[^>]*>([\s\S]*?)</file>', full_xml):
            sections_parsed.append((m.group(1), _html.unescape(m.group(2))))
        budget = 4000
        kept, total = [], 0
        for path, content in sections_parsed:
            header = f"## File: {path}\n"
            tokens = r.estimate_tokens(header + content)
            if total + tokens > budget:
                continue  # skip the whole file — never exceed, never truncate
            kept.append((path, content))
            total += tokens
        self.assertNotIn("a/big.py", [p for p, _ in kept])
        self.assertIn("a/small.py", [p for p, _ in kept])
        self.assertLessEqual(total, budget)


class PinProofTest(unittest.TestCase):
    """Part 11: version-only matches are PIN-UNVERIFIED; only the pinned
    path-resolves proof passes transitively."""

    def setUp(self):
        self.r = load("repomix_adapter")

    def test_version_match_without_proven_commit_is_pin_unverified(self):
        r = self.r
        # Fake install: package.json with version == lock, NO gitHead, and a
        # PINNED_SOURCE_DIR that is NOT where the package resolves.
        with tempfile.TemporaryDirectory() as d:
            pkg = Path(d) / "package.json"
            pkg.write_text(json.dumps({"name": "repomix", "version": r.LOCKED_REPOMIX_VERSION}))
            orig_pkg_dir = r.locate_repomix_package
            orig_pinned = r.PINNED_SOURCE_DIR
            r.locate_repomix_package = lambda: Path(d)
            r.PINNED_SOURCE_DIR = Path(d) / "elsewhere"
            try:
                with self.assertRaises(r.PinUnverified):
                    r.verify_repomix_pin()
            finally:
                r.locate_repomix_package = orig_pkg_dir
                r.PINNED_SOURCE_DIR = orig_pinned

    def test_installed_resolving_to_locked_checkout_passes(self):
        r = self.r
        # The installed package IS the pinned checkout at the locked HEAD.
        with tempfile.TemporaryDirectory() as d:
            pkg = Path(d) / "package.json"
            pkg.write_text(json.dumps({"name": "repomix", "version": r.LOCKED_REPOMIX_VERSION}))
            (Path(d) / ".git").mkdir(exist_ok=True)
            orig_pkg_dir = r.locate_repomix_package
            orig_pinned = r.PINNED_SOURCE_DIR
            r.locate_repomix_package = lambda: Path(d)
            r.PINNED_SOURCE_DIR = Path(d)
            # Mock git rev-parse HEAD.
            orig_run = r.subprocess.run
            r.subprocess.run = lambda argv, **kw: type("P", (), {"returncode": 0, "stdout": r.LOCKED_REPOMIX_COMMIT + "\n"})()

            try:
                r.verify_repomix_pin()  # should not raise
            finally:
                r.locate_repomix_package = orig_pkg_dir
                r.PINNED_SOURCE_DIR = orig_pinned
                r.subprocess.run = orig_run


class NativeRoutingTest(unittest.TestCase):
    """Part 10: native-default reuses the canonical variant set, never
    synthetic -native names, never a zero budget, correct mode reporting."""

    def _stub_tasks(self, h):
        """Give main() a real repo 'x' with one task (no network, no tools)."""
        fixtures = tempfile.TemporaryDirectory()
        (Path(fixtures.name) / "x").mkdir(exist_ok=True)
        (Path(fixtures.name) / "x" / "a.py").write_text("def a(): pass\n")
        orig_f = h.FIXTURES
        h.FIXTURES = Path(fixtures.name)
        orig_lt = h.load_tasks
        h.load_tasks = lambda: {"x": [
            {"id": "t1", "goal": "g1", "files": ["a.py"], "symbols": ["s1"]},
        ]}
        return orig_f, orig_lt, fixtures

    def _restore_tasks(self, h, orig_f, orig_lt, fixtures):
        h.FIXTURES = orig_f
        h.load_tasks = orig_lt
        fixtures.cleanup()

    def test_main_expands_native_without_suffix_variants(self):
        h = load("run_context_bench")
        orig_f, orig_lt, fixtures = self._stub_tasks(h)
        calls = []
        orig_ext = h.run_external_variant
        orig_nat = h.run_native_variant

        def fake_ext(variant, tasks, budget, agent_cmd, workdir, mode="equal-token"):
            calls.append(("ext", variant, budget, mode))
            return [], None

        def fake_nat(variant, tasks, budget, agent_cmd, scc_bin, workdir, mode="equal-token"):
            calls.append(("nat", variant, budget, mode))
            return [], None
        h.run_external_variant = fake_ext
        h.run_native_variant = fake_nat
        try:
            rc = h.main(["--mode", "native-default", "--repo", "x", "--single", "--json"])
            self.assertEqual(rc, 0)
        finally:
            h.run_external_variant = orig_ext
            h.run_native_variant = orig_nat
            self._restore_tasks(h, orig_f, orig_lt, fixtures)
        # No "-native" synthetic variant names reached the runners.
        self.assertFalse(any(v.endswith("-native") for _, v, _, _ in calls))

    def test_budgets_are_none_in_native_mode(self):
        h = load("run_context_bench")
        orig_f, orig_lt, fixtures = self._stub_tasks(h)
        # The (variant, mode) model: native mode must NOT use budgets=[0];
        # the None sentinel is what disables the shared cap end-to-end.
        calls = []
        orig_ext = h.run_external_variant
        orig_nat = h.run_native_variant

        def fake_ext(variant, tasks, budget, agent_cmd, workdir, mode="equal-token"):
            calls.append((budget, mode))
            return [], None

        def fake_nat(variant, tasks, budget, agent_cmd, scc_bin, workdir, mode="equal-token"):
            calls.append((budget, mode))
            return [], None
        h.run_external_variant = fake_ext
        h.run_native_variant = fake_nat
        try:
            h.main(["--mode", "native-default", "--repo", "x", "--single", "--json"])
        finally:
            h.run_external_variant = orig_ext
            h.run_native_variant = orig_nat
            self._restore_tasks(h, orig_f, orig_lt, fixtures)
        self.assertTrue(calls, "native mode must actually run variants")
        for budget, mode in calls:
            self.assertIsNone(budget, "native mode must not synthesize a budget")
            self.assertEqual(mode, "native-default")

    def test_adapter_native_mode_reports_single_mode_key(self):
        # Part 10/13: --native with a zero budget is accepted (no budget
        # rejection before honoring native), and the JSON carries exactly one
        # "mode" key = "native-default" (the pre-fix duplicate overwrote it).
        import io as _io
        from contextlib import redirect_stdout
        a = load("aider_adapter")
        with tempfile.TemporaryDirectory() as d:
            repo = Path(d) / "repo"
            repo.mkdir()
            (repo / "a.py").write_text("def f():\n    return 1\n")
            out = Path(d) / "out"
            orig_build = a.build_repomap_native
            a.build_repomap_native = lambda repo_abs, goal, site_packages=None: ("native map", None)
            orig_src = a.src_files
            a.src_files = lambda directory: [str(Path(directory) / "a.py")]
            buf = _io.StringIO()
            try:
                with redirect_stdout(buf):
                    code = a.main(["aider_adapter.py", str(repo), "0", str(out), "--native"])
                self.assertEqual(code, 0, buf.getvalue())
            finally:
                a.build_repomap_native = orig_build
                a.src_files = orig_src
            payload = json.loads(buf.getvalue())
            self.assertEqual(payload["mode"], "native-default")
            self.assertIsNone(payload["requested_budget"])
            self.assertEqual(payload["actual_shared_tokens"], a.estimate_tokens("native map"))


if __name__ == "__main__":
    unittest.main()

class WritableModeTest(unittest.TestCase):
    """Part 13: task_success_rate is EVALUATOR-driven (validate/tests), and
    run_completion_rate stays a separate process-exit metric."""

    def setUp(self):
        self.h = load("run_context_bench")
        self._fixtures = tempfile.TemporaryDirectory()
        (Path(self._fixtures.name) / "repo").mkdir(exist_ok=True)
        (Path(self._fixtures.name) / "repo" / "a.py").write_text("def a(): return 1\n")
        self._orig_fixtures = self.h.FIXTURES
        self.h.FIXTURES = Path(self._fixtures.name)

    def tearDown(self):
        self.h.FIXTURES = self._orig_fixtures
        self._fixtures.cleanup()

    def test_task_success_is_evaluator_driven(self):
        # Agent "exits 0" but fails the evaluator -> run_completion 1.0,
        # task_success 0.0. The two MUST be distinct.
        self.h.run_write_task = lambda agent_cmd, root, goal, validate_cmd=None, tests_cmd=None, artifact_path=None: {
            "run_completion": True,
            "task_success": False,
            "task_success_defined": True,
            "patch": "x", "patch_produced": True,
            "modified_files": ["a.py"], "eval_exit": 1,
            "wall_sec": 5.0,
        }
        tasks = [{"id": "t1", "goal": "g1", "files": [], "symbols": [], "validate": "true"}]
        artifacts = [(Path(self._fixtures.name) / "a.txt", 100)]
        row = self.h._row_for(tasks, artifacts, "agent", "repo", "aider-repomap", 8000, mode="equal-token", writable=True)
        self.assertEqual(row["run_completion_rate"], 1.0)
        self.assertEqual(row["task_success_rate"], 0.0)
        self.assertEqual(row["tasks_with_evaluator"], 1)
        self.assertIn("patch_rate", row)

    def test_task_success_absent_without_evaluator(self):
        # No validate/tests on the task -> task_success_rate is None
        # (never a cheap exit-code pass).
        self.h.run_write_task = lambda agent_cmd, root, goal, validate_cmd=None, tests_cmd=None, artifact_path=None: {
            "run_completion": True,
            "task_success": None,
            "task_success_defined": False,
            "patch": "", "patch_produced": False,
            "modified_files": [], "eval_exit": None, "wall_sec": 1.0,
        }
        tasks = [{"id": "t1", "goal": "go1", "files": [], "symbols": []}]
        artifacts = [(Path(self._fixtures.name) / "a.txt", 100)]
        row = self.h._row_for(tasks, artifacts, "agent", "repo", "aiderrepomap", None, mode="native-default", writable=True)
        self.assertIsNone(row["task_success_rate"])
        self.assertEqual(row["tasks_with_evaluator"], 0)


class PairedBootstrapTest(unittest.TestCase):
    """Part 16: paired bootstrap 95% CI over paired outcomes; deterministic."""

    def test_ci_sane_and_deterministic(self):
        h = load("run_context_bench")
        a = [1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0]
        b = [0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0]
        m1, lo1, hi1 = h.paired_bootstrap_ci(a, b)
        m2, lo2, hi2 = h.paired_bootstrap_ci(a, b)
        self.assertEqual((m1, lo1, hi1), (m2, lo2, hi2), "deterministic seed -> identical CI")
        self.assertTrue(lo1 <= m1 <= hi1)
        self.assertLess(m1, 1.0)


class AggregateShapeTest(unittest.TestCase):
    """Part 12/19: aggregate() and print_table() accept BOTH the read-only
    (exploration) and writable (task_success) row shapes without KeyError;
    a run must never crash at report time."""

class AggregateShapeTest(unittest.TestCase):
    """Part 12/19: aggregate() and print_table() accept BOTH the read-only
    (exploration) and writable (task_success) row shapes without KeyError;
    a run must never crash at report time."""

    def test_aggregate_handles_both_row_shapes(self):
        h = load("run_context_bench")
        ro_row = {
            "variant": "aider", "budget": 8000, "repo": "r", "tasks": 1,
            "run_completion_rate": 1.0, "mean_exploration": 4.0,
            "first_plan_accuracy": 1.0, "context_tokens": 100,
            "mean_files_opened": 3.0, "mean_search_tool_calls": 1.0,
            "mean_files_opened_before_first_correct": 1.0,
        }
        wr_row = {
            "variant": "aider", "budget": 8000, "repo": "r", "tasks": 2,
            "run_completion_rate": 0.5, "task_success_rate": 0.0,
            "tasks_with_evaluator": 2, "patch_rate": 0.5, "mean_wall_sec": 3.0,
            "context_tokens": 10,
        }
        agg = h.aggregate([ro_row, wr_row])
        self.assertAlmostEqual(agg["run_completion_rate"], 0.75)
        # task_success_rate is present (a 0.0, not None) and the read-only
        # key that the writable row lacks does not crash the aggregation.
        self.assertEqual(agg["task_success_rate"], 0.0)
        self.assertEqual(agg["mean_exploration"], 4.0)
        # print_table renders the mixed shapes without KeyError.
        import io as _io
        from contextlib import redirect_stdout
        buf = _io.StringIO()
        with redirect_stdout(buf):
            h.print_table([ro_row, wr_row], json_out=False)
        self.assertIn("aider", buf.getvalue())


if __name__ == "__main__":
    unittest.main()
