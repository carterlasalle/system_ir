"""Thin Python SDK for the ``scc`` (System Context Compiler) CLI.

Every method shells out to the ``scc`` binary with ``--root <cwd>`` and
``--json``, and parses the emitted context pack. The binary is resolved from
the ``bin`` constructor argument, then the ``SCC_BIN`` environment variable,
then ``scc`` on PATH. A non-zero exit raises :class:`SCCError` with the
process's stderr.
"""

import json
import os
import subprocess
from typing import Any, Dict, List, Optional

# trace:v1 id=impl.scc.sdk.python work=WORK-SCC-014 satisfies=REQ-SCC-IR


class SCCError(Exception):
    """Raised when the ``scc`` CLI exits with a non-zero status."""


# trace:exempt reason=internal-detail  # thin CLI subprocess wrapper, not repo behavior
class SCC:
    """Client for the ``scc`` CLI (thin subprocess wrapper)."""

    def __init__(self, bin: Optional[str] = None, cwd: Optional[str] = None) -> None:
        self._bin = bin or os.environ.get("SCC_BIN") or "scc"
        self._cwd = cwd or os.getcwd()

    def _run(self, args: List[str]) -> subprocess.CompletedProcess:
        proc = subprocess.run(
            [self._bin, "--root", self._cwd, *args],
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode != 0:
            message = proc.stderr.strip() or "{} exited with code {}".format(
                self._bin, proc.returncode
            )
            raise SCCError(message)
        return proc

    def _run_json(self, args: List[str]) -> Dict[str, Any]:
        proc = self._run(args)
        return json.loads(proc.stdout)

    def systemOverview(self) -> Dict[str, Any]:
        """Compile the system overview capsule."""
        return self._run_json(["overview", "--json"])

    def taskContext(
        self,
        goal: str,
        files: Optional[List[str]] = None,
        symbols: Optional[List[str]] = None,
        tokenBudget: Optional[int] = None,
    ) -> Dict[str, Any]:
        """Compile a task context pack for a goal."""
        args = ["context", "task", goal]
        if files:
            args.extend(["--files", " ".join(files)])
        if symbols:
            args.extend(["--symbols", " ".join(symbols)])
        if tokenBudget is not None:
            args.extend(["--budget", str(tokenBudget)])
        args.append("--json")
        return self._run_json(args)

    def componentContext(self, id: str) -> Dict[str, Any]:
        """Compile the context pack for one component (by id or name)."""
        return self._run_json(["context", "component", id, "--json"])

    def flowContext(self, id: str) -> Dict[str, Any]:
        """Compile the context pack for one flow (by id or name)."""
        return self._run_json(["context", "flow", id, "--json"])

    def impactContext(
        self, files: Optional[List[str]] = None, symbols: Optional[List[str]] = None
    ) -> Dict[str, Any]:
        """Compile an impact analysis pack for a set of files/symbols."""
        args = ["impact"]
        if files:
            args.extend(files)
        if symbols:
            args.extend(["--symbols", " ".join(symbols)])
        args.append("--json")
        return self._run_json(args)

    def verifyContext(self) -> Dict[str, Any]:
        """Run the freshness/evidence verification.

        ``scc verify`` has no JSON mode, so the pack is synthesized from its
        markdown output.
        """
        proc = self._run(["verify"])
        revision = ""
        for line in proc.stdout.splitlines():
            if line.startswith("Revision:"):
                revision = line.split(":", 1)[1].strip()
                break
        return {
            "kind": "verify",
            "repository_revision": revision,
            "content": proc.stdout,
            "entity_ids": [],
            "evidence_summary": {},
            "warnings": [],
            "tokens": 0,
            "budget": 0,
            "truncated": False,
        }

    # trace:exempt reason=internal-detail  # CLI mirror wrapper, behavior traced at impl.scc.cli
    def contextStartup(self, budget: Optional[int] = None) -> Dict[str, Any]:
        """Compile the fused session-startup artifact (Atlas + Surface +
        coverage + omissions).

        ``scc context startup`` has no JSON mode, so the pack is synthesized
        from its markdown output.
        """
        args = ["context", "startup"]
        if budget is not None:
            args.extend(["--budget", str(budget)])
        proc = self._run(args)
        return {
            "kind": "startup",
            "repository_revision": "",
            "content": proc.stdout,
            "entity_ids": [],
            "evidence_summary": {},
            "warnings": [],
            "tokens": 0,
            "budget": budget or 0,
            "truncated": False,
        }

    # trace:exempt reason=internal-detail  # CLI mirror wrapper, behavior traced at impl.scc.cli
    def surfaceMap(
        self, goal: Optional[str] = None, budget: Optional[int] = None
    ) -> Dict[str, Any]:
        """Compile the System Surface Map, global or task-personalized.

        ``scc surface`` has no JSON mode, so the pack is synthesized from
        its markdown output.
        """
        args = ["surface"]
        if goal:
            args.extend(["--task", goal])
        if budget is not None:
            args.extend(["--budget", str(budget)])
        proc = self._run(args)
        return {
            "kind": "surface",
            "repository_revision": "",
            "content": proc.stdout,
            "entity_ids": [],
            "evidence_summary": {},
            "warnings": [],
            "tokens": 0,
            "budget": budget or 0,
            "truncated": False,
        }

    # trace:exempt reason=internal-detail  # CLI mirror wrapper, behavior traced at impl.scc.cli
    def structuralSource(
        self,
        files: Optional[List[str]] = None,
        goal: Optional[str] = None,
        budget: Optional[int] = None,
    ) -> Dict[str, Any]:
        """Compile the Structural Source representation of files (explicit
        ``files`` or the files lexically matched to a ``goal``).

        ``scc context structural`` has no JSON mode, so the pack is
        synthesized from its markdown output.
        """
        args = ["context", "structural"]
        if files:
            args.extend(["--files", " ".join(files)])
        if goal:
            args.extend(["--task", goal])
        if budget is not None:
            args.extend(["--budget", str(budget)])
        proc = self._run(args)
        return {
            "kind": "structural",
            "repository_revision": "",
            "content": proc.stdout,
            "entity_ids": [],
            "evidence_summary": {},
            "warnings": [],
            "tokens": 0,
            "budget": budget or 0,
            "truncated": False,
        }

    def index(self) -> Dict[str, bool]:
        """Index the repository (idempotent; incremental after the first run)."""
        self._run(["index"])
        return {"ok": True}
