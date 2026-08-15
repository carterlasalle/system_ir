"""Hermes plugin contract test — runs without Hermes installed.

Verifies the plugin package against a mock `ctx` exactly as the Hermes
plugin docs describe (`register(ctx)` with register_tool/register_skill),
then exercises every handler against a real indexed fixture repo.

Usage:
    cd plugins/hermes
    SCC_BIN=/path/to/scc python3 test_plugin.py
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).parent
sys.path.insert(0, str(HERE))

import scc  # the plugin package (plugins/hermes/scc/__init__.py -> import scc?)


class MockCtx:
    def __init__(self):
        self.tools = {}
        self.skills = {}

    def register_tool(self, name, toolset, schema, handler):
        self.tools[name] = {"toolset": toolset, "schema": schema, "handler": handler}

    def register_skill(self, name, path):
        self.skills[name] = path


def make_fixture_repo():
    root = tempfile.mkdtemp()
    repo = Path(root) / "repo"
    repo.mkdir()
    (repo / "a.py").write_text("def helper():\n    return 1\n")
    subprocess.run(
        [os.environ.get("SCC_BIN", "scc"), "index", "--quiet"],
        cwd=repo,
        check=True,
        capture_output=True,
    )
    return repo


# trace:v1 id=test.scc.hermes.plugin verifies=REQ-SCC-IR exercises=impl.scc.hermes
class HermesPluginTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.ctx = MockCtx()
        scc.register(cls.ctx)
        try:
            cls.repo = make_fixture_repo()
        except (FileNotFoundError, subprocess.CalledProcessError):
            cls.repo = None

    # trace:exempt reason=internal-detail  # plugin contract test; behavior traced at impl.scc.cli
    def test_registers_ten_tools_and_skill(self):
        self.assertEqual(
            set(self.ctx.tools),
            {
                "system_overview",
                "system_atlas",
                "task_context",
                "component_context",
                "flow_context",
                "impact_context",
                "verify_context",
                "system_context",
                "surface_map",
                "structural_source",
            },
        )
        self.assertIn("scc-system-context", self.ctx.skills)

    def test_schemas_are_valid(self):
        for name, t in self.ctx.tools.items():
            self.assertEqual(t["schema"]["name"], name)
            self.assertIn("description", t["schema"])

    def test_handlers_against_real_repo(self):
        if self.repo is None:
            self.skipTest("scc binary not available")
        old = os.getcwd()
        os.chdir(self.repo)
        try:
            for name in ("system_overview", "system_atlas", "verify_context"):
                out = json.loads(self.ctx.tools[name]["handler"]({}))
                self.assertNotIn("error", out, out)
            atlas = json.loads(self.ctx.tools["system_atlas"]["handler"]({}))
            self.assertIn("ARCHITECTURE", str(atlas), atlas)
            out = json.loads(self.ctx.tools["task_context"]["handler"]({"goal": "helper"}))
            self.assertIn("TASK", str(out), out)
            out = json.loads(self.ctx.tools["impact_context"]["handler"]({"files": ["a.py"]}))
            self.assertIn("RISK", str(out), out)
        finally:
            os.chdir(old)


if __name__ == "__main__":
    unittest.main()
