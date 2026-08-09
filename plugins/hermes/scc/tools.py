"""SCC tool handlers — thin wrappers over the local `scc` CLI.

Every handler: receives args (dict), does the work, returns a JSON string,
never raises (Hermes plugin contract).
"""

import json
import os
import shutil
import subprocess


def _scc_bin():
    return os.environ.get("SCC_BIN") or shutil.which("scc") or "scc"


def _run(args):
    """Run scc with --json and return parsed output."""
    try:
        proc = subprocess.run(
            [_scc_bin(), *args, "--json"],
            capture_output=True,
            text=True,
            timeout=120,
        )
    except FileNotFoundError:
        return {
            "error": "scc binary not found — install System Context Compiler or set SCC_BIN"
        }
    except subprocess.TimeoutExpired:
        return {"error": "scc timed out"}
    if proc.returncode != 0:
        return {"error": proc.stderr.strip() or "scc failed"}
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError:
        return {"error": "scc returned non-JSON output"}


def system_overview(args: dict, **kwargs) -> str:
    return json.dumps(_run(["overview"]))


def task_context(args: dict, **kwargs) -> str:
    cmd = ["context", "task", args.get("goal", "")]
    if args.get("files"):
        cmd += ["--files", " ".join(args["files"])]
    if args.get("symbols"):
        cmd += ["--symbols", " ".join(args["symbols"])]
    if args.get("token_budget"):
        cmd += ["--budget", str(args["token_budget"])]
    return json.dumps(_run(cmd))


def component_context(args: dict, **kwargs) -> str:
    return json.dumps(_run(["context", "component", args.get("component", "")]))


def flow_context(args: dict, **kwargs) -> str:
    return json.dumps(_run(["context", "flow", args.get("flow", "")]))


def impact_context(args: dict, **kwargs) -> str:
    cmd = ["impact"]
    if args.get("files"):
        cmd += list(args["files"])
    if args.get("symbols"):
        cmd += ["--symbols", " ".join(args["symbols"])]
    if args.get("diff"):
        cmd += ["--diff", args["diff"]]
    return json.dumps(_run(cmd))


def verify_context(args: dict, **kwargs) -> str:
    return json.dumps(_run(["verify"]))
