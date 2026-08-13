# Holdout diagnosis — why blind repos score near zero, and what the atlas/compilers must learn

Generated from `scc bench atlas --holdout --diagnose` (release binary, Wave 9 final,
2026-08-12). Every missed holdout ground-truth key was classified by `--diagnose`'s
deterministic store→flows→components→text ladder into PARSER / EXTRACTOR / RESOLUTION /
COMPILER / PROJECTION / ALIAS. The per-repo gap lines are the regeneration source for
`benchmarks/results/holdout-v1.txt` / `ground-truth-gaps.md` (holdout half).

## Holdout gap-kind histogram (all sections, all 20 repos)

```
  ALIAS        29
  COMPILER    259
  EXTRACTOR   456
  PROJECTION   32
  RESOLUTION    1
```

The dominant miss is **EXTRACTOR (456)** — a parsed language, but the semantic fact never
reaches the store (symbols/entrypoints/flows/contracts the extractors do not emit). The
second is **COMPILER (259)** — the fact is in the store but the compilers never wire it into
a component or flow. Projection (32) is small; ALIAS (29) is small; RESOLUTION (1) is
negligible. No PARSER gaps at all — every holdout repo is in a language with an enabled
extractor.

## Why the "zero" repos score near zero (requests / ripgrep / guava / zod / zerolog / axios)

The original v1 report showed several holdout repos at exactly 0.000 (requests, ripgrep,
guava, zod, zerolog, axios, helm). With the current Wave 9 binary (first-class contracts +
invocation surfaces + java enabled) those repos are no longer exactly zero but still far
below the 0.5 gate. Per-repo recall (overall = mean of architecture+entrypoints+behavior+
state_authority+contracts) and the miss breakdown:

| repo   | lang | overall | arch | entry | behav | state | contr | ALIAS | COMPILER | EXTRACTOR | PROJ |
|--------|------|--------:|-----:|------:|------:|------:|------:|------:|---------:|----------:|-----:|
| axios  | TS   | 0.120 | 0.333 | 0.100 | 0.000 | 0.167 | 0.000 | 2 | 7 | 31 | 2 |
| cobra  | Go   | 0.122 | 0.000 | 0.000 | 0.000 | 0.429 | 0.182 | 0 | 22 | 22 | 0 |
| helm   | Go   | 0.070 | 0.000 | 0.067 | 0.000 | 0.286 | 0.000 | 0 | 23 | 36 | 1 |
| guava  | Java | 0.575 | 1.000 | 1.000 | 0.000 | 0.875 | 0.000 | 0 | 8 | 19 | 1 |
| netty  | Java | 0.555 | 1.000 | 0.900 | 0.000 | 0.875 | 0.000 | 0 | 6 | 18 | 0 |
| requests| Py  | 0.301 | 0.875 | 0.200 | 0.000 | 0.429 | 0.000 | 4 | 10 | 21 | 0 |
| ripgrep| Rust | 0.086 | 0.000 | 0.286 | 0.000 | 0.143 | 0.000 | 6 | 20 | 21 | 0 |
| zod    | TS   | 0.270 | 0.750 | 0.000 | 0.000 | 0.600 | 0.000 | 3 | 8 | 19 | 7 |
| zerolog| Go   | 0.176 | 0.000 | 0.167 | 0.000 | 0.714 | 0.000 | 0 | 21 | 24 | 1 |

Classifying the misses for each:

### axios (TS, 0.120) — EXTRACTOR-dominated
- **EXTRACTOR (31)**: the entire request pipeline (`Axios.request -> _request ->
  mergeConfig -> dispatchRequest`, `dispatchRequest -> transformRequest -> adapter(request)`,
  `InterceptorManager.forEach -> chain execution`) and all config contracts
  (`axios({method:'get'...})`, `.then(response => response.data)`, `interceptor use(fn,fn)`,
  `cancelToken: new CancelToken(cb)`) never reach the store as flows/contracts. The TS
  extractor emits class/export symbols but not inter-method call chains or the config-object
  contract surface.
- **COMPILER (7)**: `lib/axios.js`, `core/Axios`, `core/dispatchRequest`,
  `core/InterceptorManager`, `core/AxiosHeaders`, `core/AxiosError` are in the store as
  symbols but never compiled into components.
- **PROJECTION (2) / ALIAS (2)**: small.

### ripgrep (Rust, 0.086) — workspace/multi-crate + COMPILER
- **COMPILER (20)**: every crate (`crates/core`, `crates/core/flags`, `crates/searcher`,
  `crates/grep`, `crates/matcher`, `crates/printer`, `crates/ignore`, `crates/cli`) is in the
  store but the workspace crates are never compiled into components — the atlas renders a
  single `CRATES` component (the whole crates/ dir), not per-crate components.
- **EXTRACTOR (21)**: CLI contracts (`rg <pattern>`, `-e/--regexp`, `-i/--ignore-case`,
  `--json`, `-A/-B/-C`, `-g/--glob`) and the search pipeline flows (`main -> flags::parse ->
  run -> SearchWorkerBuilder -> SearchWorker.search`) are not emitted; `HiArgs`/`LowArgs`
  state is in the store but only as annotate/Debug, never compiled to state authority.
- **ALIAS (6)**: `architecture:crates/core`, `crates/core/flags`, `HiArgs`, `LowArgs`,
  `SearchWorker`, `Stats` — these are genuine gaps (the atlas renders `CRATES` for the whole
  dir, not `crates/core`; `HiArgs` is a `#[derive(Debug)]` annotation, not a state owner), so
  the GT keys are correct as written and were NOT changed.

### guava (Java, 0.575) — behavior + contracts, and near-zero on contracts
- arch/entry/state all ≥0.875; the entire loss is **behavior (0.000) + contracts (0.000)**.
- **EXTRACTOR (19)**: every behavior chain (`CacheBuilder.build -> LocalCache creation ->
  CacheLoader`, `LocalCache.get -> segment lookup -> load`, `Hashing.md5 -> hashFunction.
  newHasher -> hash`) and every factory contract (`ImmutableList.of(a,b,c)`,
  `CacheBuilder.newBuilder().maximumSize(n).build(loader)`, `cache.get(key, callable)`,
  `Preconditions.checkArgument(cond,msg)`) is absent from the store — the Java extractor
  emits class/method symbols and route-style contracts but not intra-class call chains or
  factory-method contract forms.
- **COMPILER (8)**: `behavior` steps are in the store as methods but not wired into flows.

### zod (TS, 0.270) — entrypoints + contracts
- **EXTRACTOR (19) + PROJECTION (7)**: all entrypoint factories (`z.string`, `z.number`,
  `z.object`, `z.union`, `z.enum`, `schema.parse`, `z.infer`) and contracts
  (`z.string().min(1)`, `z.object({name: z.string()})`) absent. The v4/classic facade is
  under `packages/zod/src/v4/classic/external.ts` — the extractor sees the classic exports but
  not the `z.*` surface.
- **COMPILER (8)**: `globalRegistry`, `errorTree` in store, never compiled to state authority.

### zerolog (Go, 0.176) — flat package + COMPILER-heavy
- **COMPILER (21)**: `log.go`, `context.go`, `console.go`, `array.go`, `globals.go` are files
  in the store but never compiled into components; `Logger`, `Context`, `Event` symbols never
  become state authority.
- **EXTRACTOR (24)**: all entrypoints (`zerolog.New(writer)`, `log.Info`, `event.Msg`,
  `logger.With()`, `zerolog.SetGlobalLevel`) and behavior chains (`Info().Msg(...) -> newEvent
  -> Level -> write JSON`) absent — Go package-level method calls and builder chains not
  extracted.
- state_authority 0.714 is the only strong layer.

### requests (Python, 0.301) — behavior + contracts
- **EXTRACTOR (21)**: the request lifecycle (`Session.request -> Session.prepare_request ->
  Session.send -> HTTPAdapter.send`, `HTTPAdapter.send -> urllib3.urlopen -> build Response`,
  `Session.send -> resolve_redirects`) and the kwarg contracts (`timeout=(connect,read)`,
  `allow_redirects=False`, `stream=True`, `verify=False`, `auth=('user','pass')`) absent.
- **COMPILER (10)**: `requests.api`, `Session.get/post` in store, not compiled to components.
- **ALIAS (4)**: `merge_setting`, `Response.content`, `default_headers`, `codes` — genuine
  gaps (they exist as exports/property annotations, not as state authority / flows); NOT
  changed.

### cobra / helm (Go, 0.122 / 0.070) — CLI + COMPILER
- cobra: **COMPILER 22 / EXTRACTOR 22**. helm: **COMPILER 23 / EXTRACTOR 36**. Both lose
  architecture (cobra 0.000, helm 0.000) because the command-package files (`command.go`,
  `cmd/helm`, `pkg/cmd`) are in the store but never compiled into components; the CLI
  contracts (`helm install <release> <chart>`, `rootCmd.Execute()`, `cobra.Command{Use:...}`,
  `cmd.Flags().StringVarP(...)`) and subcommand behavior chains are EXTRACTOR gaps.

## ALIAS-key fixes applied (atlas-rendered forms — same QA discipline as the dev corpus)

`--diagnose` flagged 29 ALIAS items (fact present in the rendered atlas but the structured
layer's spelling/aliases did not reconcile it). Each was checked against the atlas's actual
structured haystack (owns claims + data stores for state_authority; entrypoint names for
entrypoints; flow names/steps for behavior; contract ops for contracts; component
names/purposes/implementation for architecture). Of the 29, exactly **one** was a genuine
atlas-rendered-form alias the ground-truth key should adopt; the other 28 are genuine gaps
(fact exists only as README prose, an annotation, or an export — the structured layer
carries no reconcilable spelling, so changing the key would be gaming, not QA).

Fixed in `benchmarks/holdout-ground-truth/click.md`:

- `state_authority:Context.meta` → `Context._meta` — the atlas owns the backing field
  (`src owns Context._meta (EXTRACTED)`); the GT named the public property. Aligned to the
  atlas-rendered owns claim.

The 28 ALIAS items deliberately NOT changed (all genuine gaps — documented here so the
compiler agent can close them properly instead of the GT being weakened):

- axios: `state_authority:mergeConfig`, `state_authority:AxiosHeaders` (exports, not state).
- black: `state_authority:Report` (a flow step, not an owns claim).
- chi: `state_authority:RouteContext` (public_api function, not state authority).
- click: `entrypoints:click.command/group/option/argument` (decorators render as
  `annotation: click.command` in the contracts text, not as entrypoints).
- django: `state_authority:django.apps.apps` (registry instance appears in flows/contracts,
  not as an owns claim).
- rayon: `entrypoints:par_iter`, `state_authority:ThreadPool`, `state_authority:WorkerThread`
  (trait method / exports, not entrypoints or owned state).
- react: `state_authority:workInProgress` (a flow step).
- requests: `behavior:merge_setting`, `state_authority:Response.content`,
  `state_authority:default_headers`, `contracts:codes` (exports/property annotations).
- ripgrep: `architecture:crates/core`, `architecture:crates/core/flags`,
  `state_authority:HiArgs/LowArgs/SearchWorker/Stats` (workspace-dir component granularity /
  derive annotations).
- vue: `architecture:packages/compiler-core`, `state_authority:renderer` (dir-level
  component granularity / a flow reference).
- zod: `architecture:packages/zod`, `state_authority:globalRegistry`,
  `state_authority:errorTree` (dir-level component / exports).

## Generalization gap list — what the atlas/compilers must learn to score these repos

Feed this to the compiler agent's roadmap. Each is the *root* of a large EXTRACTOR/COMPILER
block; closing it lifts whole sections, not single keys.

1. **EXTRACTOR: intra-class / inter-method call chains.** requests, axios, zerolog, guava,
   netty, black all lose `behavior` (0.000 everywhere) because method-to-method call chains
   (`A.b -> A.c -> D.e`) never become flow steps. The extractors emit symbols and (for some
   languages) direct callee edges, but the compilers never promote an inter-method call chain
   into a named behavior flow. This is the single largest lever — behavior is 0.000 for every
   holdout repo.

2. **EXTRACTOR: builder/factory-method contract surface.** guava (`ImmutableList.of`,
   `CacheBuilder.newBuilder().maximumSize(n).build(loader)`), axios (`axios({method...})`),
   zerolog (`zerolog.New(os.Stderr).With().Timestamp().Logger()`), click (`@click.command()`),
   cobra (`cobra.Command{Use:...}`, `cmd.Flags().StringVarP(...)`) contracts are absent. The
   atlas renders `config:`/`http:` contract ops but not fluent-builder/factory contract forms.

3. **COMPILER: workspace / per-crate and per-package component granularity.** ripgrep
   (`crates/core`, `crates/matcher`), helm (`cmd/helm`, `pkg/action`), cobra (`command.go`),
   vue/zod (`packages/compiler-core`, `packages/zod`) — the atlas compiles the whole
   `crates/`/`packages/`/`pkg/` dir into ONE component (`CRATES`, `PACKAGES`) instead of one
   component per crate/package. All 20 arch misses for ripgrep/helm/cobra and the vue/zod arch
   misses trace here. This is the second-largest lever for the near-zero repos.

4. **COMPILER: symbols → state authority.** `HiArgs`/`LowArgs`/`SearchWorker`/`Stats`
   (ripgrep), `ThreadPool`/`WorkerThread` (rayon), `workInProgress` (react), `Report` (black),
   `globalRegistry`/`errorTree` (zod), `mergeConfig`/`AxiosHeaders` (axios) are in the store
   as symbols/annotations but never claimed as owned state, so state_authority stays well
   below 1.0 for the monorepo/workspace repos.

5. **COMPILER: CLI files → components.** cobra `command.go`/`args.go`, helm `cmd/helm` +
   `pkg/cmd`, ripgrep `crates/core` render nothing in architecture because the CLI package
   files are indexed as files/symbols but not grouped into a command component.

6. **RESOLUTION (1)** and **PROJECTION (32)** are minor; not worth a dedicated roadmap item.

7. **ALIAS (29)** — of which 28 are genuine (the fact is not in the target layer under any
   reconcilable spelling); only the click `Context._meta` backing-field form was a GT-side
   alias. Do not "close" the other 28 by editing GT — fix the underlying extraction/compile
   instead (items 1–5).

## Updated holdout recall

Re-run `scc bench atlas --holdout` after the single ALIAS fix (`click.md`) — `holdout-v1.txt`
regenerated:

```
layer                     dev    holdout        gap
architecture            0.815      0.322     -0.493
entrypoints             0.248      0.422     +0.174
behavior                0.115      0.000     -0.115
state_authority         0.000      0.450     +0.450
contracts               0.187      0.051     -0.136
overall (gate)          0.273      0.249     -0.024
verdict: BORDERLINE (holdout 0.249 vs dev 0.273; tolerance 0.050)
```

The ALIAS fix moved click from overall 0.244 → **0.278** (state_authority 0.667 → 0.833;
the `Context._meta` owns claim now reconciles). Holdout overall is 0.249 — no longer any
exactly-0.000 repo (all 20 score 0.070–0.575). The gap vs dev is within the 0.05 tolerance
band (BORDERLINE, not OVERFIT). The structural gap list above is the actionable output for
the compiler agent — closing items 1–3 (call-chain flows, builder/factory contracts,
per-crate/per-package components) is what lifts the near-zero repos (axios 0.120, cobra
0.122, helm 0.070, ripgrep 0.086, zerolog 0.176) toward the gate.
