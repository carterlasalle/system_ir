# Wave-15 external benchmark suite

Compares seven context variants (A–H) for coding agents on the fixture
corpus, driven through the benchagent protocol (`$SCC_GOAL` env, repo cwd,
JSONL tool-event stream) and scored on the spec §72 columns: **variant,
success rate, mean exploration, first-plan accuracy, context tokens**.

## Variants

| id               | context artifact                                                      | runs where |
|------------------|-----------------------------------------------------------------------|------------|
| `raw`            | none (task only)                                                      | native     |
| `aider-repomap`  | aider RepoMap (pinned)                                                | python     |
| `repomix-compress` | repomix pack `--compress`, equal-token (complete files, never mid-file) | python  |
| `scc-atlas`      | `scc context startup --budget N`, atlas section only                  | native     |
| `scc-surface`    | `scc surface`                                                         | native     |
| `scc-atlas-surface` | full `scc context startup --budget N`                              | native     |
| `scc-full`       | startup + `scc context task` delta + structural source for the task's matched files | native |

Equal-token mode: the budgets (4000 / 8000 / 16000 / 24000) apply to the
**context artifacts**, not the agent prompt.

## Pins

`benchmarks/external-lock.json` pins the external tools:

- aider: `Aider-AI/aider` @ `5dc9490bb35f9729ef2c95d00a19ccd30c26339c`
- repomix: `yamadashy/repomix` @ `e3b15a406ed78d8a463620a032a059ce911bfc0e`

Install:

```sh
pip install "git+https://github.com/Aider-AI/aider.git@5dc9490bb35f9729ef2c95d00a19ccd30c26339c"
npm install -g "github:yamadashy/repomix#e3b15a406ed78d8a463620a032a059ce911bfc0e"   # or the npx fallback in the adapter
```

## What runs natively vs. needs the external tools

- **Native (in the `scc` binary, `scc bench external`)**: `raw`, `scc-atlas`,
  `scc-surface`, `scc-atlas-surface`, `scc-full`. Artifacts are generated
  through the production CLI itself (`scc context startup --budget N`,
  `scc surface`, `scc context task`, structural source) into the workdir,
  then each task runs the benchagent protocol. No python or external tool
  needed.
- **Python harness (`run_context_bench.py`)** drives the full matrix; for
  the native variants it shells out to `scc bench external --json` per
  (variant, budget, repo), for the external-tool variants it calls the
  adapters and runs the same event protocol itself.
- **External tools** (`aider-repomap`, `repomix-compress`) require the
  pinned aider/repomix installs. `scc bench external --variant
  aider-repomap` delegates to the python harness.

## SKIPPED-UNINSTALLED

When an external tool is missing, its adapter exits **2** with
`{"ok": false, "error": "SKIPPED-UNINSTALLED: ..."}` on stdout. The harness
reports the variant row as `SKIPPED-UNINSTALLED` and continues with the
other variants; the delegated `scc bench external` arm exits 0 with that
status in the row (so a matrix run missing aider still completes).

## Usage

```sh
# full A-H matrix over every ground-truth repo (needs an agent CLI)
python3 benchmarks/external/run_context_bench.py \
  --agent-cmd 'codex exec --json --sandbox read-only --skip-git-repo-check --ephemeral --color never -C . -'

# one variant, one budget, one repo, JSON rows
python3 benchmarks/external/run_context_bench.py --variant scc-atlas-surface \
  --budget 8000 --repo cli-service --json

# native single-variant run through the scc binary
scc bench external --variant scc-atlas-surface --repo cli-service --budget 8000 \
  --cmd 'codex exec --json --sandbox read-only --skip-git-repo-check --ephemeral --color never -C . -' \
  --workdir /tmp/ext-bench --json
```

The agent command receives the prompt (context artifact + task) on stdin
and the goal in `$SCC_GOAL` (the benchagent protocol; see
`benchmarks/run_agent_bench.sh` for the codex form).

## Ground truth

`benchmarks/external/ground-truth.yaml` holds per-repo tasks for fixture
repos not in the canonical `benchmarks/tasks.json` (currently `cli-service`),
using the Wave-14 §75 vocabulary: `public_surfaces`, `important_types`,
`implementation_landmarks` (the localization ground truth = `files`),
plus `symbols` for first-plan keys. The Rust side merges tasks.json + this
file; the python harness does the same.

## Output

Human table (default) or JSON rows (`--json`):

```text
variant            success  mean_exploration  first_plan_acc  tokens  status
scc-atlas-surface    1.000              3.00           1.000    2067
aider-repomap          —                 —               —        —     SKIPPED-UNINSTALLED (aider CLI not found on PATH)
```

`context_tokens` is the deterministic chars/4 estimate of the artifact
(0 for `raw`). `mean_exploration` = files opened + search calls + graph
queries per task.
