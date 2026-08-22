#!/usr/bin/env python3
"""Aider-adapter fairness tests (Part G/H/N).

Casing preservation for mentioned_idents (the pinned aider matches exactly,
case-sensitively), mentioned_fnames extraction from literal task paths, and
the equal-token search contract of build_repomap (mocked RepoMap: the map
generated must fit the shared chars/4 budget without mid-truncation).
"""

import importlib.util
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load_adapter():
    spec = importlib.util.spec_from_file_location(
        "aider_adapter", HERE / "aider_adapter.py"
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class MentionedIdentsTest(unittest.TestCase):
    def setUp(self):
        self.a = load_adapter()

    def test_pascal_case_survives(self):
        idents = self.a.mentioned_idents("change IncidentEngine retryPolicy")
        self.assertIn("IncidentEngine", idents)
        self.assertIn("retryPolicy", idents)

    def test_lowercase_variant_added_not_replacing(self):
        idents = self.a.mentioned_idents("change IncidentEngine retryPolicy in src/engine.ts")
        # original casing first-class...
        self.assertIn("IncidentEngine", idents)
        self.assertIn("retryPolicy", idents)
        # ...and the lowercase twin is available for fuzzy hits, never as a
        # replacement for the original
        self.assertIn("incidentengine", idents)
        self.assertLess(idents.index("IncidentEngine"), idents.index("incidentengine"))

    def test_path_token_preserved_in_idents_and_fnames(self):
        goal = "change IncidentEngine retryPolicy in src/engine.ts"
        idents = self.a.mentioned_idents(goal)
        self.assertIn("src/engine.ts", idents)
        fnames = self.a.mentioned_fnames(goal)
        self.assertEqual(fnames, ["src/engine.ts"])

    def test_fnames_only_literal_paths(self):
        # ground truth must NEVER be inferred: no path-like string in the
        # task -> no fnames
        self.assertEqual(self.a.mentioned_fnames("rename the transcript field"), [])
        # extension-only mention counts; invented names do not appear
        self.assertEqual(self.a.mentioned_fnames("fix parser.py handling"), ["parser.py"])

    def test_stopwords_and_flags(self):
        idents = self.a.mentioned_idents("--verbose add the error handling to the widget")
        self.assertNotIn("verbose", [i.lower() for i in idents])
        self.assertIn("widget", idents)
        self.assertNotIn("error", [i.lower() for i in idents])  # grammar word


class _FakeRepoMap:
    """Records constructor args; generates a map whose size tracks
    map_tokens linearly so the equal-token search has something to solve."""

    calls = []

    def __init__(self, map_tokens=None, root=None, main_model=None, io=None, refresh=None):
        type(self).calls.append(map_tokens)
        self.map_tokens = map_tokens if map_tokens is not None else 1024

    def get_repo_map(self, chat_files, other_files, mentioned_idents=None, mentioned_fnames=None):
        n = max(1, int(self.map_tokens)) * 4  # chars = tokens*4 -> estimate == map_tokens
        return "x" * n


class EqualTokenSearchTest(unittest.TestCase):
    def setUp(self):
        import types
        self.a = load_adapter()
        # Bypass pin checks by monkeypatching the guard.
        self._orig_installed = self.a.installed_aider_commit
        self.a.installed_aider_commit = lambda site_packages=None: self.a.LOCKED_AIDER_COMMIT
        # Inject a fake aider package so build_repomap's imports resolve and
        # the search runs against a deterministic map generator.
        fake_repo_map = types.ModuleType("aider.repomap")
        fake_repo_map.RepoMap = _FakeRepoMap
        _FakeRepoMap.calls = []
        fake_io = types.ModuleType("aider.io")
        fake_io.InputOutput = lambda **kw: object()
        fake_models = types.ModuleType("aider.models")
        fake_models.Model = lambda name: None
        fake_pkg = types.ModuleType("aider")
        fake_pkg.__path__ = []
        for name, mod in {
            "aider": fake_pkg,
            "aider.repomap": fake_repo_map,
            "aider.io": fake_io,
            "aider.models": fake_models,
        }.items():
            sys.modules[name] = mod
        self._fake_modules = (fake_pkg, fake_repo_map, fake_io, fake_models)

    def tearDown(self):
        self.a.installed_aider_commit = self._orig_installed
        for name in ("aider", "aider.repomap", "aider.io", "aider.models"):
            sys.modules.pop(name, None)

    def test_equal_token_map_never_exceeds_budget(self):
        budget = 500
        text, used = self.a.build_repomap("/nonexistent-repo", budget, "", site_packages=str(HERE))
        self.assertLessEqual(self.a.estimate_tokens(text), budget)
        # The search converged on a native parameter within a sane band.
        self.assertGreater(used, 0)

    def test_native_mode_reports_unnormalized_cost(self):
        text, used = self.a.build_repomap_native("/nonexistent-repo", "", site_packages=str(HERE))
        self.assertIsNone(used)
        self.assertGreater(self.a.estimate_tokens(text), 0)


if __name__ == "__main__":
    unittest.main()
