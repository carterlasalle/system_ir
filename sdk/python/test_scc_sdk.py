"""Integration tests for the scc-sdk against a real ``scc`` binary and a
throwaway fixture repository. Skipped when no ``scc`` binary is available
(via the SCC_BIN environment variable or PATH)."""

import os
import shutil
import stat
import tempfile
import unittest
from pathlib import Path

from scc_sdk import SCC, SCCError

A_PY = """def add(a, b):
    return a + b

class Calculator:
    def multiply(self, x, y):
        return x * y
"""

B_PY = """from a import add, Calculator

result = add(1, 2)
calc = Calculator()
prod = calc.multiply(3, 4)
"""


def resolve_scc_bin():
    from_env = os.environ.get("SCC_BIN")
    if from_env:
        return from_env
    return shutil.which("scc")


BIN = resolve_scc_bin()


# trace:v1 id=test.scc.sdk.python verifies=REQ-SCC-IR exercises=impl.scc.sdk.python
@unittest.skipUnless(BIN, "scc binary not found (set SCC_BIN or add scc to PATH)")
class TestSCCSDK(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="scc-sdk-py-")
        (Path(cls.tmp) / ".git").mkdir()
        (Path(cls.tmp) / "a.py").write_text(A_PY)
        (Path(cls.tmp) / "b.py").write_text(B_PY)
        cls.scc = SCC(bin=BIN, cwd=cls.tmp)
        cls.scc.index()

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def test_index_returns_ok(self):
        result = self.scc.index()
        self.assertEqual(result, {"ok": True})

    def test_system_overview_content_identifies_repository(self):
        pack = self.scc.systemOverview()
        self.assertEqual(pack["kind"], "overview")
        self.assertIn("IDENTITY", pack["content"])
        self.assertIsInstance(pack["entity_ids"], list)

    # trace:exempt reason=internal-detail  # sdk integration test; behavior traced at impl.crates-scc-cli-src-commands.build-task-context
    def test_task_context_has_entity_ids_array(self):
        artifact = self.scc.taskContext("transcript")
        self.assertEqual(artifact["pack"]["kind"], "task")
        self.assertIsInstance(artifact["pack"]["entity_ids"], list)
        self.assertIn("Goal: transcript", artifact["pack"]["content"])
        self.assertIsInstance(artifact["delta"], str)
        # delta_ids is ALWAYS serialized now (empty array, never omitted) —
        # the exact public contract the TS type declares.
        self.assertIsInstance(artifact["delta_ids"], list)
        self.assertIsInstance(artifact["token_count"], int)

    # trace:exempt reason=internal-detail  # sdk integration test; behavior traced at impl.crates-scc-cli-src-commands.build-task-context
    def test_task_context_honors_options(self):
        artifact = self.scc.taskContext(
            "add numbers", files=["a.py", "b.py"], symbols=["add"], tokenBudget=500
        )
        self.assertIn("Explicit files: a.py, b.py", artifact["pack"]["content"])
        self.assertIn("Explicit symbols: add", artifact["pack"]["content"])

    def test_component_context_resolves_component(self):
        pack = self.scc.componentContext("root")
        self.assertEqual(pack["kind"], "component")
        self.assertTrue(pack["entity_ids"])
        self.assertIn("RESPONSIBILITY", pack["content"])

    def test_flow_context_resolves_flow(self):
        pack = self.scc.flowContext("architecture")
        self.assertEqual(pack["kind"], "flow")
        self.assertTrue(pack["entity_ids"])
        self.assertIn("STEPS", pack["content"])

    def test_impact_context_returns_impact_pack(self):
        pack = self.scc.impactContext(files=["a.py"], symbols=["add"])
        self.assertEqual(pack["kind"], "impact")
        self.assertIn("RISK", pack["content"])

    def test_verify_context_content_reports_freshness(self):
        pack = self.scc.verifyContext()
        self.assertEqual(pack["kind"], "verify")
        self.assertIn("FRESHNESS", pack["content"])

    # trace:exempt reason=internal-detail  # sdk integration test; behavior traced at impl.scc.cli
    def test_context_startup_renders_fused_artifact(self):
        pack = self.scc.contextStartup()
        self.assertEqual(pack["kind"], "startup")
        self.assertIn("# SCC SYSTEM CONTEXT", pack["content"])
        self.assertIn("## SYSTEM ATLAS", pack["content"])
        self.assertIn("## SYSTEM SURFACE MAP", pack["content"])

    # trace:exempt reason=internal-detail  # sdk integration test; behavior traced at impl.scc.cli
    def test_surface_map_renders_global_and_personalized(self):
        pack = self.scc.surfaceMap()
        self.assertEqual(pack["kind"], "surface")
        self.assertIn("SCC SYSTEM SURFACE MAP", pack["content"])
        personalized = self.scc.surfaceMap(goal="add numbers")
        self.assertIn("task-personalized: add numbers", personalized["content"])

    # trace:exempt reason=internal-detail  # sdk integration test; behavior traced at impl.scc.cli
    def test_structural_source_renders_files_and_goal(self):
        pack = self.scc.structuralSource(files=["a.py"])
        self.assertEqual(pack["kind"], "structural")
        self.assertIn("source: a.py:L", pack["content"])
        by_goal = self.scc.structuralSource(goal="multiply calculator")
        self.assertIn("representation:", by_goal["content"])

    # trace:exempt reason=internal-detail  # sdk parity test; behavior traced at impl.crates-scc-cli-src-commands.build-task-context
    def test_task_context_mirrors_cli_json_exactly(self):
        """Parity: the SDK's taskContext() must return the CLI's
        `scc context task --json` output verbatim (nested artifact shape,
        no flattening) — the CLI JSON, Python SDK, and TypeScript SDK all
        expose the same {pack, delta, delta_ids, token_count} contract for
        the same fixture/goal."""
        import json
        import subprocess

        goal = "transcript"
        sdk_artifact = self.scc.taskContext(goal)

        cli = subprocess.run(
            [BIN, "--root", str(self.tmp), "context", "task", goal, "--json"],
            capture_output=True,
            text=True,
            check=True,
        )
        cli_artifact = json.loads(cli.stdout)

        # The SDK MUST expose the CLI shape: top-level pack (not flattened).
        self.assertEqual(sorted(sdk_artifact.keys()), sorted(cli_artifact.keys()))
        self.assertEqual(sdk_artifact["pack"], cli_artifact["pack"])
        self.assertEqual(sdk_artifact["delta"], cli_artifact["delta"])
        if "delta_ids" in cli_artifact:
            self.assertEqual(sdk_artifact["delta_ids"], cli_artifact["delta_ids"])
        self.assertEqual(sdk_artifact["token_count"], cli_artifact["token_count"])
        # And the nested pack fields are the flat pack contract.
        self.assertEqual(sdk_artifact["pack"]["kind"], "task")
        self.assertIn("Goal: transcript", sdk_artifact["pack"]["content"])

    def test_nonzero_exit_raises_scc_error(self):
        fake_bin = Path(self.tmp) / "fake-scc"
        fake_bin.write_text("#!/bin/sh\necho 'boom: exploded' >&2\nexit 3\n")
        fake_bin.chmod(fake_bin.stat().st_mode | stat.S_IEXEC)
        client = SCC(bin=str(fake_bin), cwd=self.tmp)
        with self.assertRaises(SCCError) as ctx:
            client.systemOverview()
        self.assertIn("boom: exploded", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
