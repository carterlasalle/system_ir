# External context-benchmark results (Wave 15)

## Variant matrix (cli-service, mock agent)
Run: `python3 benchmarks/external/run_context_bench.py --repo cli-service --json`
Raw JSON per (variant, budget) in results/; context artifacts in artifacts/.

- raw / aider-repomap / repomix-compress: exploration metrics require the
  real agent harness; aider+repomix report SKIPPED-UNINSTALLED until the
  pinned tools are installed (benchmarks/external-lock.json).
- scc-* variants run in-process via `scc bench external` / the harness;
  context-token columns are real artifact sizes.

## Localization ablation (artifact-only recall, spec §57-58)
Task: "expose a health check route on the router"
Targets: cli.rs, build_router, health, Router (ground-truth.yaml)

| variant        | recall | tokens | density (recall/1k tok) |
|----------------|--------|--------|--------------------------|
| atlas          | 3/4    | 7983   | 0.38                     |
| surface        | 1/4    | 4323   | 0.23                     |
| atlas+surface  | 3/4    | 7983   | 0.38                     |
| task-delta     | 4/4    | 299    | 13.4                     |

Task-delta (task PPR + novelty filter) localizes 13x denser than the
atlas for a single task — the Aider-fusion thesis measured. Surface-only
recall is low because build_router/health are internal (not exported
surfaces); the surface renders public API as designed.
