"""SCC tool handlers — thin wrappers over the local `scc` CLI.

Every handler: receives args (dict), does the work, returns a JSON string,
never raises (Hermes plugin contract).
"""

import json
import os
import shutil
import subprocess

# trace:v1 id=impl.scc.hermes.tools work=WORK-SCC-014 satisfies=REQ-SCC-IR


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


# trace:exempt reason=internal-detail  # CLI wrapper helper; behavior traced at impl.scc.cli
def _run_text(args):
    """Run scc WITHOUT --json and return the raw stdout text.

    Used by the text-artifact tools (startup/surface/structural have no
    --json mode); failures return the same error dict shape as `_run`.
    """
    try:
        proc = subprocess.run(
            [_scc_bin(), *args],
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
    return {"content": proc.stdout}


def system_overview(args: dict, **kwargs) -> str:
    return json.dumps(_run(["overview"]))


def system_atlas(args: dict, **kwargs) -> str:
    cmd = ["atlas"]
    if args.get("token_budget"):
        cmd += ["--budget", str(args["token_budget"])]
    return json.dumps(_run(cmd))


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


# trace:exempt reason=internal-detail  # CLI mirror wrapper; behavior traced at impl.scc.cli
def system_context(args: dict, **kwargs) -> str:
    """The fused session-startup artifact (Atlas + Surface + coverage + omissions)."""
    cmd = ["context", "startup"]
    if args.get("token_budget"):
        cmd += ["--budget", str(args["token_budget"])]
    return json.dumps(_run_text(cmd))


# trace:exempt reason=internal-detail  # CLI mirror wrapper; behavior traced at impl.scc.cli
def surface_map(args: dict, **kwargs) -> str:
    """The System Surface Map, global or task-personalized."""
    cmd = ["surface"]
    if args.get("goal"):
        cmd += ["--task", args["goal"]]
    if args.get("token_budget"):
        cmd += ["--budget", str(args["token_budget"])]
    return json.dumps(_run_text(cmd))


# trace:exempt reason=internal-detail  # CLI mirror wrapper; behavior traced at impl.scc.cli
def structural_source(args: dict, **kwargs) -> str:
    """Structural Source for explicit files or a goal-resolved file set."""
    cmd = ["context", "structural"]
    if args.get("files"):
        cmd += ["--files", " ".join(args["files"])]
    if args.get("goal"):
        cmd += ["--task", args["goal"]]
    if args.get("token_budget"):
        cmd += ["--budget", str(args["token_budget"])]
    return json.dumps(_run_text(cmd))
