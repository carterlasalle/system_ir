#!/usr/bin/env python3
"""Wave 15 external benchmark suite — the A-H variant harness.

Runs the context variants over the ground-truth tasks and prints the
spec §72 table (variant, success rate, mean exploration, first-plan
accuracy, context tokens).

Variants (A–H + the Wave-15 PPR ablation matrix):
    raw               no context artifact (the agent sees only the task)
    aider-repomap     pinned aider RepoMap (@ external-lock.json),
                      personalized with the task goal (mentioned_idents)
    repomix-compress  pinned repomix pack --compress, equal-token mode
    scc-atlas         `scc atlas --budget N` — the FULL budget goes to the
                      Atlas (no 13:7 startup split handicap)
    scc-surface       scc surface
    scc-atlas-surface full scc context startup --budget N
    scc-full          startup + scc context task delta + structural source
                      for the task's GOAL-selected files (never task.files —
                      ground truth is scoring-only)
    lexical / global-ppr / task-ppr / ppr-mmr / ppr-quotas /
    ppr-optimizer     PPR ablation matrix (harness-level mode flags; see
                      scc_cli::benchagent::SurfaceAblation)

Equal-token mode: budgets 4000/8000/16000/24000 apply to the CONTEXT
ARTIFACTS, not the agent prompt. ONE shared tokenizer counts every
variant's artifact the same way: deterministic chars/4. scc-full enforces
the FINAL artifact budget on the concatenated startup + task-delta +
structural text (startup N/2, task N/4, structural the remainder), never
per piece.

Agent-run protocol (reuses benchagent.rs's runner): the scc-native variants
delegate to `scc bench external`, which drives each task through the
ground-truth corpus with the goal in $SCC_GOAL, parses the agent's JSONL
event stream (files opened, search calls, graph queries, wrong-first,
first-correct), and emits a metric row per variant. The external-tool
variants (aider/repomix) are driven by this harness directly with the same
event protocol; when the tool is not installed the adapter exits 2 and the
variant is reported SKIPPED-UNINSTALLED; when the installed tool does not
match the pinned commit the adapter exits 3 and the variant is reported
PIN-MISMATCH; when the tool's commit cannot be proven (no gitHead, no
pinned checkout — a version-only match is NOT proof) the adapter exits 4
and the variant is reported PIN-UNVERIFIED, excluded from the official
showdown metric rows.

Usage:
    run_context_bench.py [--variant V] [--budget N] [--repo R]
                         [--agent-cmd CMD] [--scc-bin PATH] [--workdir DIR]
                         [--json] [--help]

Default: the full variant x budget matrix over every ground-truth repo.
--agent-cmd: the agent command; the prompt (context artifact + task goal)
is piped to it on stdin and $SCC_GOAL carries the goal (the benchagent
protocol; see benchmarks/run_agent_bench.sh for the codex form).
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

VARIANTS = [
    "raw",
    "aider-repomap",
    "repomix-compress",
    "scc-atlas",
    "scc-surface",
    "scc-atlas-surface",
    "scc-full",
    # Wave-15 PPR ablation matrix (harness-level mode flags; see
    # scc_cli::benchagent::SurfaceAblation)
    "lexical",
    "global-ppr",
    "task-ppr",
    "ppr-mmr",
    "ppr-quotas",
    "ppr-optimizer",
]
NATIVE_VARIANTS = [
    "raw",
    "scc-atlas",
    "scc-surface",
    "scc-atlas-surface",
    "scc-full",
    "lexical",
    "global-ppr",
    "task-ppr",
    "ppr-mmr",
    "ppr-quotas",
    "ppr-optimizer",
]
EXTERNAL_VARIANTS = ["aider-repomap", "repomix-compress"]
BUDGETS = [4000, 8000, 16000, 24000]
DEFAULT_BUDGET = 8000

DEFAULT_AGENT_CMD = (
    "codex exec --json --sandbox read-only --skip-git-repo-check "
    "--ephemeral --color never -C . -"
)

ROOT = Path(__file__).resolve().parent.parent.parent  # repo root
BENCHMARKS = ROOT / "benchmarks"
FIXTURES = ROOT / "fixtures"
LOCK = BENCHMARKS / "external-lock.json"

# The pinned-tools venv (see benchmarks/external/README.md): aider is
# installed there (PEP 668 keeps it out of the Homebrew pythons). The
# harness invokes the python adapters with this interpreter so `import
# aider` resolves to the pinned install. Override with SCC_BENCH_VENV.
BENCH_VENV = Path(os.environ.get("SCC_BENCH_VENV", str(Path.home() / ".scc-bench-venv")))


def bench_python():
    """The interpreter that has the pinned aider install (the bench venv),
    falling back to the running interpreter when the venv is absent."""
    py = BENCH_VENV / "bin" / "python"
    return str(py) if py.exists() else sys.executable

SOURCE_EXTS = (
    ".py", ".pyi", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".rs",
    ".go", ".java", ".kt", ".kts", ".rb", ".php", ".c", ".h", ".cpp",
    ".hpp", ".cc", ".cs", ".dart", ".proto", ".txt", ".json", ".toml",
    ".yaml", ".yml", ".md", ".sh", ".sql", ".html", ".css", ".vue",
    ".svelte", ".swift", ".lua", ".xml", ".gradle", ".dockerfile",
)


# --------------------------------------------------------------------------
# ground truth
# --------------------------------------------------------------------------

def parse_scalar(s):
    """Scalars in the ground-truth file: inline [a, b] lists or plain
    strings (quotes stripped)."""
    s = s.strip()
    if s.startswith("[") and s.endswith("]"):
        inner = s[1:-1].strip()
        if not inner:
            return []
        return [t.strip().strip("'\"") for t in inner.split(",") if t.strip()]
    return s.strip().strip("'\"")


def parse_ground_truth_yaml(text):
    """Tiny YAML-subset parser for benchmarks/external/ground-truth.yaml —
    the schema we own (2-space indent: repo / tasks / dash items / fields,
    inline lists, # comments). Avoids a PyYAML dependency."""
    repos = {}
    repo = None
    task = None
    for raw in text.splitlines():
        line = raw.split("#", 1)[0].rstrip()
        if not line.strip():
            continue
        indent = len(line) - len(line.lstrip(" "))
        content = line.strip()
        if indent <= 2 and content.endswith(":") and not content.startswith("-"):
            if indent == 0:
                continue  # top-level headers (version, repos)
            repo = content[:-1]
            repos[repo] = []
            task = None
            continue
        if indent == 4 and content == "tasks:":
            continue
        if indent == 6 and content.startswith("- "):
            task = {}
            repos[repo].append(task)
            rest = content[2:].strip()
            if ":" in rest:
                key, _, val = rest.partition(":")
                task[key.strip()] = parse_scalar(val)
            continue
        if indent == 8 and task is not None and ":" in content:
            key, _, val = content.partition(":")
            task[key.strip()] = parse_scalar(val)
    return repos


def load_tasks():
    """Merge benchmarks/tasks.json (canonical 21-task corpus) with
    benchmarks/external/ground-truth.yaml (extra fixture repos such as
    cli-service). Returns {repo: [task]} with task = {id, goal, files,
    symbols}."""
    merged = {}

    with open(BENCHMARKS / "tasks.json") as fh:
        corpus = json.load(fh)
    for task in corpus["tasks"]:
        gt = task.get("ground_truth", {})
        merged.setdefault(task["repo"], []).append(
            {
                "id": task["id"],
                "goal": task["goal"],
                "files": list(gt.get("files", [])),
                "symbols": list(gt.get("symbols", [])),
            }
        )

    yaml_path = BENCHMARKS / "external" / "ground-truth.yaml"
    if yaml_path.exists():
        doc = parse_ground_truth_yaml(yaml_path.read_text())
        for repo, tasks in doc.items():
            for task in tasks:
                files = list(task.get("implementation_landmarks") or task.get("public_surfaces") or [])
                symbols = list(task.get("symbols") or []) + list(task.get("important_types") or [])
                merged.setdefault(repo, []).append(
                    {
                        "id": task["id"],
                        "goal": task["goal"],
                        "files": files,
                        "symbols": symbols,
                    }
                )
    return merged


def filter_repos(tasks, repo):
    if repo is None:
        return tasks
    if repo not in tasks:
        return {}
    return {repo: tasks[repo]}


# --------------------------------------------------------------------------
# benchagent event protocol (port of benchagent.rs parse_event_line)
# --------------------------------------------------------------------------

def tool_kind(tool):
    t = tool.lower()
    if t in ("rg", "ag", "ack", "fd", "find") or "grep" in t or "glob" in t or "search" in t:
        return "search"
    if t in ("cat", "sed", "less", "more", "head", "tail", "wc", "nl", "open") or "read" in t or "view" in t:
        return "read"
    return "other"


def is_graph_tool(tool):
    t = tool.lower()
    return "graph" in t or "gitnexus" in t


def tool_from_command(command):
    toks = command.split()
    if not toks:
        return ""
    first = toks[0].strip("'\"")
    is_shell = (first.startswith("/") and first.endswith("sh")) or first in ("zsh", "bash", "sh")
    if is_shell:
        for tok in toks[1:]:
            tok = tok.strip("'\"")
            if tok and not tok.startswith("-"):
                return tok
        return "shell"
    return first


def normalize_path(tok, root):
    tok = tok.rstrip("/")
    if not tok:
        return None
    abs_path = tok if tok.startswith("/") else os.path.join(root, tok)
    try:
        rel = os.path.relpath(abs_path, root)
    except ValueError:
        return None
    if rel == "." or rel.startswith(".."):
        return None
    rel = rel.replace(os.sep, "/")
    if os.path.exists(abs_path) or rel.endswith(SOURCE_EXTS):
        return rel
    return None


def paths_from_command(command, root):
    out = []
    for raw in command.split():
        tok = raw.strip("'\"")
        if not tok or tok in (".", "..") or tok.startswith("-"):
            continue
        if any(ch in tok for ch in "$|&;<>*?`="):
            continue
        p = normalize_path(tok, root)
        if p:
            out.append(p)
    return out


def paths_from_args(args, root):
    out = []
    for key in ("file_path", "path", "file", "filename"):
        val = args.get(key)
        if isinstance(val, str):
            p = normalize_path(val, root)
            if p:
                out.append(p)
    for val in args.get("paths", []) or []:
        if isinstance(val, str):
            p = normalize_path(val, root)
            if p:
                out.append(p)
    return out


def parse_event(line, root):
    """Tolerant JSONL event parser (codex --json and generic tool_use
    shapes). Returns None for non-events; else (paths, kind, graph)."""
    stripped = line.strip()
    if not stripped.startswith("{"):
        return None
    try:
        value = json.loads(stripped)
    except ValueError:
        return None
    etype = value.get("type")
    if etype == "item.started":
        return None
    item = value.get("item") if etype == "item.completed" else value
    if not isinstance(item, dict):
        return None
    itype = item.get("type")
    if itype == "command_execution":
        command = item.get("command", "") or ""
        tool = tool_from_command(command)
        return (paths_from_command(command, root), tool_kind(tool), is_graph_tool(tool))
    if itype == "mcp_tool_call":
        tool = item.get("tool", "") or "mcp_tool_call"
        args = item.get("arguments") if isinstance(item.get("arguments"), dict) else {}
        return (paths_from_args(args, root), tool_kind(tool), is_graph_tool(tool))
    if itype == "tool_use":
        tool = item.get("name", "") or "tool_use"
        args = item.get("input") if isinstance(item.get("input"), dict) else {}
        return (paths_from_args(args, root), tool_kind(tool), is_graph_tool(tool))
    return None


# --------------------------------------------------------------------------
# agent runs (external-tool variants; the scc variants use `scc bench
# external`, which runs the same protocol natively in benchagent.rs)
# --------------------------------------------------------------------------

def copy_tree(src, dst):
    dst = Path(dst)
    dst.mkdir(parents=True, exist_ok=True)
    for entry in Path(src).iterdir():
        if entry.name == ".scc":
            continue
        target = dst / entry.name
        if entry.is_dir():
            copy_tree(entry, target)
        else:
            shutil.copy2(entry, target)


def run_task_via_protocol(agent_cmd, artifact_path, goal, gt_files, plan_keys, root):
    """Run one task with the benchagent protocol: SCC_GOAL env, repo cwd,
    artifact + goal piped to the agent on stdin, JSONL events parsed from
    stdout. Returns the per-task metric dict."""
    import shlex
    import time
    quoted = shlex.quote(str(artifact_path))
    variant_cmd = (
        f"CTX=$(cat {quoted} 2>/dev/null || true); "
        f"printf 'SCC CONTEXT:\\n%s\\n\\nTASK: %s\\n' \"$CTX\" \"$SCC_GOAL\" "
        f"| sh -c {shlex.quote(agent_cmd)}"
    )
    started = time.monotonic()
    proc = subprocess.run(
        ["sh", "-c", variant_cmd],
        cwd=root,
        env={**os.environ, "SCC_GOAL": goal},
        capture_output=True,
        text=True,
        timeout=1800,
    )
    output = (proc.stdout or "") + (proc.stderr or "")
    files_surfaced = sum(1 for f in gt_files if f in output)

    opened = set()
    wrong_first = set()
    search = read = graph = total = 0
    first_correct_ms = None
    gt_seen = False
    plan_buf = []
    plan_done = False
    for line in (proc.stdout or "").splitlines():
        if not plan_done:
            plan_buf.append(line)
        event = parse_event(line, root)
        if event is not None:
            plan_done = True
            paths, kind, graph_flag = event
            total += 1
            if kind == "search":
                search += 1
            elif kind == "read":
                read += 1
            if graph_flag:
                graph += 1
            for p in paths:
                opened.add(p)
                if not gt_seen and p not in gt_files:
                    wrong_first.add(p)
            if not gt_seen and any(p in gt_files for p in paths):
                gt_seen = True
        if not gt_seen and any(f in line for f in gt_files):
            gt_seen = True
            if first_correct_ms is None:
                first_correct_ms = int((time.monotonic() - started) * 1000)
    plan_text = "\n".join(plan_buf)
    first_plan_correct = any(k in plan_text for k in plan_keys)

    return {
        "exit_ok": proc.returncode == 0,
        "files_surfaced": files_surfaced,
        "files_total": len(gt_files),
        "files_opened": len(opened),
        "search_tool_calls": search,
        "read_tool_calls": read,
        "graph_tool_calls": graph,
        "total_tool_calls": total,
        "wrong_first_locations": len(wrong_first),
        "first_correct_ms": first_correct_ms,
        "first_plan_correct": first_plan_correct,
    }


def run_external_variant(variant, tasks, budget, agent_cmd, workdir):
    """aider-repomap / repomix-compress over the tasks. Returns (rows,
    skipped) where rows is a list of per-repo metric dicts and skipped is
    None, or a dict {"status": ..., "error": ...} for the first
    SKIPPED-UNINSTALLED / PIN-MISMATCH outcome.

    Artifacts are per-task: aider personalizes the repo map with the task
    goal (mentioned_idents), the same goal the SCC task variants
    personalize with — fair Aider-vs-task-SCC."""
    rows = []
    skipped = None
    artifacts_dir = workdir / "artifacts" / variant / str(budget)
    artifacts_dir.mkdir(parents=True, exist_ok=True)
    for repo, repo_tasks in sorted(tasks.items()):
        fixture = FIXTURES / repo
        adapter = BENCHMARKS / "external" / ("aider_adapter.py" if variant == "aider-repomap" else "repomix_adapter.py")
        per_task = []
        for task in repo_tasks:
            argv = [bench_python(), str(adapter), str(fixture), str(budget), str(artifacts_dir)]
            if variant == "aider-repomap":
                argv += ["--goal", task["goal"]]
            elif variant == "repomix-compress":
                argv.append("--compress")
            proc = subprocess.run(argv, capture_output=True, text=True, timeout=1800)
            try:
                payload = json.loads(proc.stdout or "{}")
            except ValueError:
                payload = {"ok": False, "error": f"adapter output not JSON: {proc.stdout[:200]}"}
            if proc.returncode == 2:
                skipped = {"status": "SKIPPED-UNINSTALLED", "error": payload.get("error", "tool not installed")}
                return rows, skipped
            if proc.returncode == 3:
                skipped = {"status": "PIN-MISMATCH", "error": payload.get("error", "installed tool does not match the lock")}
                return rows, skipped
            if proc.returncode == 4:
                # PIN-UNVERIFIED: the tool is installed and its version
                # matches the lock but the COMMIT cannot be proven (no
                # gitHead, no pinned checkout). Reported as a distinct
                # status, never as a passing pin; excluded from the
                # official showdown metric rows.
                skipped = {"status": "PIN-UNVERIFIED", "error": payload.get("error", "installed tool commit cannot be proven against the lock")}
                return rows, skipped
            if not payload.get("ok"):
                skipped = {"status": "FAILED", "error": payload.get("error", "adapter failed")}
                return rows, skipped
            per_task.append((Path(payload["artifact"]), int(payload.get("tokens", 0))))

        row = _row_for(repo_tasks, per_task, agent_cmd, repo, variant, budget)
        rows.append(row)
    return rows, skipped


def _row_for(repo_tasks, artifacts, agent_cmd, repo, variant, budget):
    """Run the repo's tasks through the agent protocol in fixture copies,
    one goal-personalized artifact per task. `artifacts` is a list of
    (artifact_path, tokens) aligned with `repo_tasks`; the row's
    context_tokens is the mean over tasks (scc-full-style)."""
    results = []
    tokens = []
    with tempfile.TemporaryDirectory(prefix="scc-ext-") as tmp:
        tmp = Path(tmp)
        for task, (artifact, tok) in zip(repo_tasks, artifacts):
            root = tmp / task["id"]
            copy_tree(FIXTURES / repo, root)
            plan_keys = list(task["files"]) + list(task["symbols"])
            results.append(
                run_task_via_protocol(agent_cmd, artifact, task["goal"], task["files"], plan_keys, root)
            )
            tokens.append(tok)
    n = max(len(results), 1)
    mean_tokens = sum(tokens) // max(len(tokens), 1) if tokens else 0
    return {
        "variant": variant,
        "budget": budget,
        "repo": repo,
        "tasks": len(results),
        "success_rate": sum(1 for r in results if r["exit_ok"]) / n,
        "mean_exploration": sum(
            r["files_opened"] + r["search_tool_calls"] + r["graph_tool_calls"] for r in results
        ) / n,
        "first_plan_accuracy": sum(1 for r in results if r["first_plan_correct"]) / n,
        "context_tokens": mean_tokens,
        "mean_files_opened": sum(r["files_opened"] for r in results) / n,
        "mean_search_tool_calls": sum(r["search_tool_calls"] for r in results) / n,
        "mean_files_opened_before_first_correct": sum(r["wrong_first_locations"] for r in results) / n,
    }


# --------------------------------------------------------------------------
# scc-native variants -> `scc bench external`
# --------------------------------------------------------------------------

def run_native_variant(variant, tasks, budget, agent_cmd, scc_bin, workdir):
    rows = []
    for repo in sorted(tasks):
        argv = [
            scc_bin, "bench", "external",
            "--variant", variant,
            "--repo", repo,
            "--budget", str(budget),
            "--cmd", agent_cmd,
            "--workdir", str(workdir),
            "--json",
        ]
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=3600)
        if proc.returncode != 0:
            raise RuntimeError(
                f"scc bench external {variant}/{repo} failed (exit {proc.returncode}): "
                f"{proc.stderr.strip()[:500]}"
            )
        try:
            rows.append(json.loads(proc.stdout))
        except ValueError:
            raise RuntimeError(f"scc bench external {variant}/{repo} returned non-JSON: {proc.stdout[:300]}")
    return rows, None


# --------------------------------------------------------------------------
# matrix + output
# --------------------------------------------------------------------------

def aggregate(rows):
    n = max(len(rows), 1)
    return {
        "variant": rows[0]["variant"] if rows else "",
        "budget": rows[0]["budget"] if rows else 0,
        "repos": len(rows),
        "tasks": sum(r["tasks"] for r in rows),
        "success_rate": sum(r["success_rate"] for r in rows) / n,
        "mean_exploration": sum(r["mean_exploration"] for r in rows) / n,
        "first_plan_accuracy": sum(r["first_plan_accuracy"] for r in rows) / n,
        "context_tokens": sum(r["context_tokens"] for r in rows) / n,
        "mean_files_opened": sum(r["mean_files_opened"] for r in rows) / n,
        "mean_search_tool_calls": sum(r["mean_search_tool_calls"] for r in rows) / n,
        "mean_files_opened_before_first_correct": sum(
            r["mean_files_opened_before_first_correct"] for r in rows
        ) / n,
    }


def print_table(rows, json_out):
    if json_out:
        print(json.dumps(rows, indent=2))
        return
    if not rows:
        print("no rows")
        return
    print(f"{'variant':<18} {'success':>8} {'mean_exploration':>16} {'first_plan_acc':>14} {'tokens':>8}  status")
    for row in rows:
        status = row.get("status")
        if status:
            print(f"{row['variant']:<18} {'—':>8} {'—':>16} {'—':>14} {'—':>8}  {status} ({row.get('detail','')})")
            continue
        print(
            f"{row['variant']:<18} {row['success_rate']:>8.3f} {row['mean_exploration']:>16.2f} "
            f"{row['first_plan_accuracy']:>14.3f} {row['context_tokens']:>8.0f}"
        )


def main(argv):
    parser = argparse.ArgumentParser(
        prog="run_context_bench.py",
        description="Wave 15 external benchmark suite — A-H variant harness (§72 table).",
    )
    parser.add_argument("--variant", choices=VARIANTS, help="single variant (default: all)")
    parser.add_argument("--budget", type=int, help=f"single token budget (default: all of {BUDGETS})")
    parser.add_argument("--repo", help="restrict to one fixture repo")
    parser.add_argument("--agent-cmd", default=DEFAULT_AGENT_CMD, help="agent command (stdin = prompt, $SCC_GOAL = goal)")
    parser.add_argument("--scc-bin", default=None, help="path to the scc binary (default: `scc` on PATH)")
    parser.add_argument("--workdir", default=None, help="artifact/result workdir (default: benchmarks/results/external)")
    parser.add_argument("--json", action="store_true", help="emit JSON rows instead of the table")
    parser.add_argument("--single", action="store_true", help="one (variant, budget, repo-filtered) run; exit 2 when the external tool is missing")
    args = parser.parse_args(argv)

    tasks = load_tasks()
    tasks = filter_repos(tasks, args.repo)
    if not tasks:
        sys.stderr.write(f"error: no ground-truth tasks for repo {args.repo}\n")
        return 1

    workdir = Path(args.workdir) if args.workdir else BENCHMARKS / "results" / "external"
    workdir.mkdir(parents=True, exist_ok=True)

    scc_bin = args.scc_bin or os.environ.get("SCC_BIN") or "scc"

    variants = [args.variant] if args.variant else VARIANTS
    budgets = [args.budget] if args.budget else BUDGETS

    all_rows = []
    skipped_status = None
    for variant in variants:
        for budget in budgets:
            if variant in EXTERNAL_VARIANTS:
                rows, skipped = run_external_variant(variant, tasks, budget, args.agent_cmd, workdir)
                if skipped is not None:
                    status = skipped.get("status", "SKIPPED-UNINSTALLED")
                    skipped_status = {"variant": variant, "budget": budget, "status": status, "detail": skipped.get("error", "")}
                    if args.single:
                        print(json.dumps(skipped_status))
                        # Distinct exit codes: 2 = missing tool,
                        # 3 = demonstrable pin mismatch, 4 = unprovable pin.
                        return {"PIN-MISMATCH": 3, "PIN-UNVERIFIED": 4}.get(status, 2)
                    all_rows.append(skipped_status)
                    continue
                if args.single:
                    print(json.dumps(aggregate(rows)))
                    return 0
                all_rows.append(aggregate(rows))
            else:
                rows, skipped = run_native_variant(variant, tasks, budget, args.agent_cmd, scc_bin, workdir)
                if args.single:
                    print(json.dumps(aggregate(rows)))
                    return 0
                all_rows.append(aggregate(rows))

    print_table(all_rows, args.json)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
