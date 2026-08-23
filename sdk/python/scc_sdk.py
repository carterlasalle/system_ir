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


# trace:v1 id=impl.sdk-python-scc-sdk.scc work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
class SCC:
    """Client for the ``scc`` CLI (thin subprocess wrapper)."""

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.init work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def __init__(self, bin: Optional[str] = None, cwd: Optional[str] = None) -> None:
        self._bin = bin or os.environ.get("SCC_BIN") or "scc"
        self._cwd = cwd or os.getcwd()

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.run work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
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

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.run-json work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def _run_json(self, args: List[str]) -> Dict[str, Any]:
        proc = self._run(args)
        return json.loads(proc.stdout)

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.system-overview work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def systemOverview(self) -> Dict[str, Any]:
        """Compile the system overview capsule."""
        return self._run_json(["overview", "--json"])

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.task-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def taskContext(
        self,
        goal: str,
        files: Optional[List[str]] = None,
        symbols: Optional[List[str]] = None,
        tokenBudget: Optional[int] = None,
    ) -> Dict[str, Any]:
        """Compile the complete task context artifact for a goal: the enriched
        task pack plus its task-personalized Surface delta.

        Returns the CLI's `scc context task --json` output verbatim:
        ``{"pack": {...}, "delta": "...", "delta_ids": [...],
        "token_count": N}`` — ``pack`` is the flat task pack (keys
        ``kind``, ``content``, ``entity_ids``, ...); ``delta`` is the
        task-personalized Surface delta; ``delta_ids`` are the delta's
        rendered entry ids. Never flattened: consumers read
        ``result["pack"]["content"]``, not ``result["content"]``.
        """
        args = ["context", "task", goal]
        if files:
            args.extend(["--files", " ".join(files)])
        if symbols:
            args.extend(["--symbols", " ".join(symbols)])
        if tokenBudget is not None:
            args.extend(["--budget", str(tokenBudget)])
        args.append("--json")
        return self._run_json(args)

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.component-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def componentContext(self, id: str) -> Dict[str, Any]:
        """Compile the context pack for one component (by id or name)."""
        return self._run_json(["context", "component", id, "--json"])

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.flow-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def flowContext(self, id: str) -> Dict[str, Any]:
        """Compile the context pack for one flow (by id or name)."""
        return self._run_json(["context", "flow", id, "--json"])

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.impact-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
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

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.verify-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
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

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.context-startup work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
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

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.surface-map work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
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

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.structural-source work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def structuralSource(
        self,
        files: Optional[List[str]] = None,
        goal: Optional[str] = None,
        budget: Optional[int] = None,
    ) -> Dict[str, Any]:
        """Compile the Structural Source representation of files (explicit
        ``files`` or the files matched to a ``goal`` via the PPR->Surface
        pipeline).

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

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.index work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def index(self) -> Dict[str, bool]:
        """Index the repository (idempotent; incremental after the first run)."""
        self._run(["index"])
        return {"ok": True}
