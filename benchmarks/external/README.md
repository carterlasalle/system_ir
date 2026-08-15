# Wave-15 external benchmark suite

Compares the context variants for coding agents on the fixture corpus,
driven through the benchagent protocol (`$SCC_GOAL` env, repo cwd,
JSONL tool-event stream) and scored on the spec §72 columns: **variant,
success rate, mean exploration, first-plan accuracy, context tokens**.

## Variants

| id                | context artifact                                                          | runs where |
|-------------------|--------------------------------------------------------------------------|------------|
| `raw`             | none (task only)                                                         | native     |
| `aider-repomap`   | aider RepoMap (pinned), personalized with the task goal                  | python     |
| `repomix-compress`| repomix pack `--compress`, equal-token (complete files, never mid-file)  | python     |
| `scc-atlas`       | `scc atlas --budget N` — the FULL budget goes to the Atlas               | native     |
| `scc-surface`     | `scc surface --budget N`                                                 | native     |
| `scc-atlas-surface` | full `scc context startup --budget N`                                  | native     |
| `scc-full`        | startup + `scc context task` delta + structural source for the task's    | native     |
|                   | **goal-selected** files (never task.files — ground truth is scoring-only)|            |
| `lexical`         | PPR ablation: task surface ranked by lexical match only                  | native     |
| `global-ppr`      | PPR ablation: global PPR + lexical (`scc surface --budget N`)            | native     |
| `task-ppr`        | PPR ablation: task PPR + global PPR + lexical + criticality              | native     |
| `ppr-mmr`         | PPR ablation: task-ppr + MMR diversification                             | native     |
| `ppr-quotas`      | PPR ablation: task-ppr + MMR + per-kind quota caps                       | native     |
| `ppr-optimizer`   | PPR ablation: task-ppr + MMR + quotas + value/token optimizer            | native     |

Equal-token mode: the budgets (4000 / 8000 / 16000 / 24000) apply to the
**context artifacts**, not the agent prompt.

### Ground-truth discipline (audit fix)

`scc-full` and every structural variant select their structural source
files from the **task-personalized surface** (`scc context structural
--task "<goal>" --budget N` when the running binary supports it; otherwise
the harness implements the same selection via `scc surface --task "<goal>"`
and derives the files from the rendered entries). Ground-truth `task.files`
enters **scoring only** — the benchagent protocol compares tool events
against it; it never crosses into context construction.

### PPR ablation matrix

Every ablation mode runs the **production surface pipeline**
(`build_surface_staged` in scc-context — the same service the production
`surface`/`startup` paths route through) with exactly one stage toggled;
the ablation rows are the production rows with one stage removed, never a
harness reimplementation of ranking. The stage toggles
(`scc_cli::benchagent::SurfaceAblation::stages`):

| mode | stage toggle |
|------|--------------|
| `lexical` | every stage off (lexical-only) |
| `global-ppr` | task PPR off (global PPR + lexical) |
| `task-ppr` | full task pipeline (all stages on) |
| `ppr-mmr` | MMR diversity off |
| `ppr-quotas` | token-aware quotas off |
| `ppr-optimizer` | value/token budget optimizer off |

The artifact is the production render prefixed with a one-line mode label
(its token cost is subtracted from the budget, so equal-token discipline
holds).

## Fairness rules (equal-token mode)

- **One shared tokenizer for ALL variants**: deterministic chars/4
  (`len(text) / 4`, min 1). The scc native arm, both python adapters, and
  the harness all use the same rule, so an equal token budget means the
  same thing in every variant's artifact.
- **FINAL artifact budget enforcement**: for `scc-full`, the budget applies
  to the **concatenated** startup + task-delta + structural artifact, not
  per piece. Pieces are sized so the total never exceeds N: startup gets
  N/2, the task delta N/4, and the structural section gets the remainder
  of what the first two actually consumed; the harness re-slices the
  structural section (complete-file units) if the concatenation overshoots.
- **`scc-atlas` gets the FULL requested budget for the Atlas** (no 13:7
  startup split handicap when the variant is atlas-only): it runs
  `scc atlas --budget N`, not the atlas slice of a startup split.
- **`aider-repomap` gets the same task goal for personalization** as the
  SCC task variants: the goal's mentioned identifiers (simple word tokens)
  are passed as `mentioned_idents` to `RepoMap.get_repo_map`, so
  Aider-vs-task-SCC is a fair comparison.

## Pins

`benchmarks/external-lock.json` pins the external tools (commit + the
version declared at that commit):

- aider: `Aider-AI/aider` @ `5dc9490bb35f9729ef2c95d00a19ccd30c26339c`
  (version `0.86.3.dev53+g5dc9490bb`)
- repomix: `yamadashy/repomix` @ `e3b15a406ed78d8a463620a032a059ce911bfc0e`
  (version `1.18.0`)

### Install (pinned)

```sh
# aider — into the bench venv (PEP 668 keeps it out of Homebrew pythons)
python3.12 -m venv ~/.scc-bench-venv
~/.scc-bench-venv/bin/pip install \
  "git+https://github.com/Aider-AI/aider.git@5dc9490bb35f9729ef2c95d00a19ccd30c26339c"

# repomix — npm's git-install build path is broken (missing devDeps for the
# prepare script), so build the exact commit from a pinned checkout:
git clone https://github.com/yamadashy/repomix.git ~/.scc-bench/repomix
git -C ~/.scc-bench/repomix checkout e3b15a406ed78d8a463620a032a059ce911bfc0e
npm install --prefix ~/.scc-bench/repomix
npm run build --prefix ~/.scc-bench/repomix
npm install -g ~/.scc-bench/repomix
```

The harness resolves the bench venv for the python adapters
(`~/.scc-bench-venv`, override with `SCC_BENCH_VENV`).

### Pin verification (no silent floating)

The adapters verify the installed tool against the lock and hard-error on
any mismatch or unprovable install — a version-only match is never a pin:

- **aider**: the pip git install's `direct_url.json`
  `vcs_info.commit_id` (or the package's own `.git` HEAD for editable
  installs) must equal the locked commit. Test seam:
  `SCC_AIDER_SITE_PACKAGES` overrides the site-packages search root.
- **repomix**: the installed `package.json` `gitHead` must equal the
  locked commit; otherwise the documented install provenance is verified
  (global install of the pinned source checkout at
  `~/.scc-bench/repomix`, whose git HEAD is the locked commit, with the
  installed version equal to the source version). An install whose commit
  cannot be proven from either source — even one reporting the locked
  version — is **PIN-UNVERIFIED** (exit 4), never accepted as a pin.
  Test seams: `SCC_REPOMIX_PKG_DIR` overrides the installed package dir;
  `SCC_REPOMIX_SRC_DIR` overrides the pinned checkout dir.

Exit 2 `SKIPPED-UNINSTALLED` is only for a completely missing tool. The
harness reports each status in the variant row and continues the matrix.

## What runs natively vs. needs the external tools

- **Native (in the `scc` binary, `scc bench external`)**: `raw`,
  `scc-atlas`, `scc-surface`, `scc-atlas-surface`, `scc-full`, and the
  PPR ablation matrix (`lexical`, `global-ppr`, `task-ppr`, `ppr-mmr`,
  `ppr-quotas`, `ppr-optimizer`). Artifacts are generated through the
  production CLI itself (`scc context startup --budget N`, `scc atlas
  --budget N`, `scc surface [--task]`, structural source) into the
  workdir, then each task runs the benchagent protocol; the ablation
  matrix renders through the same production `build_surface_staged`
  service with one stage toggled (see the PPR ablation matrix section).
  No python or external tool needed.
- **Python harness (`run_context_bench.py`)** drives the full matrix; for
  the native variants it shells out to `scc bench external --json` per
  (variant, budget, repo), for the external-tool variants it calls the
  adapters (one goal-personalized artifact per task) and runs the same
  event protocol itself.
- **External tools** (`aider-repomap`, `repomix-compress`) require the
  pinned aider/repomix installs. `scc bench external --variant
  aider-repomap` delegates to the python harness.

## SKIPPED-UNINSTALLED / PIN-MISMATCH / PIN-UNVERIFIED

When an external tool is missing, its adapter exits **2** with
`{"ok": false, "error": "SKIPPED-UNINSTALLED: ..."}` on stdout. When it is
installed but demonstrably does not match the lock, the adapter exits **3**
with `{"ok": false, "error": "PIN-MISMATCH: ..."}`. When the installed
tool's commit cannot be proven against the lock (no gitHead, no pinned
checkout — a version-only match is NOT proof), the adapter exits **4**
with `{"ok": false, "error": "PIN-UNVERIFIED: ..."}`; the harness reports
it as a distinct status row, excluded from the official showdown metric
rows — it is never treated as a passing pin. The harness reports each
status in the variant row and continues with the other variants; the
delegated `scc bench external` arm exits 0 with that status in the row (so
a matrix run missing aider still completes).

## Usage

```sh
# full variant x budget matrix over every ground-truth repo (needs an agent CLI)
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
`benchmarks/run_agent_bench.sh` for the codex form). For deterministic
CI runs, use the mock agent from `crates/scc-cli/tests/external_bench.rs`
(a fixed JSONL event stream) — every variant runs under the SAME agent
command, so the only variable is the context artifact.

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
