# SCC Wave 8 Corpus — 20 real repositories

Real public repositories used for external validation of the SCC startup atlas.
Each repo lives in `benchmarks/corpus/<name>/` (shallow clone, `.git` kept — history is
needed by the co-change pipeline). The clones are gitignored (`benchmarks/corpus/*`); this
README and the ground-truth docs in `benchmarks/ground-truth/` are the committed artifacts.

## Manifest

| name | URL | language | category | ~LOC (tracked source) |
|---|---|---|---|---|
| fastapi | https://github.com/fastapi/fastapi | Python | python service | ~113k |
| flask | https://github.com/pallets/flask | Python | python service | ~18k |
| pydantic | https://github.com/pydantic/pydantic | Python | python service (lib) | ~173k |
| sqlalchemy | https://github.com/sqlalchemy/sqlalchemy | Python | db-heavy | ~632k |
| celery | https://github.com/celery/celery | Python | queue+events | ~102k |
| express | https://github.com/expressjs/express | TypeScript/JS | ts backend | ~21k |
| nest | https://github.com/nestjs/nest | TypeScript | ts backend | ~117k |
| prettier | https://github.com/prettier/prettier | TypeScript/JS | ts backend (tool) | ~164k |
| svelte | https://github.com/sveltejs/svelte | TypeScript | monorepo | ~157k |
| shadcn-ui | https://github.com/shadcn-ui/ui | TypeScript | nextjs fullstack | ~550k |
| clap | https://github.com/clap-rs/clap | Rust | rust cli | ~84k |
| bat | https://github.com/sharkdp/bat | Rust | rust cli | ~39k |
| serde | https://github.com/serde-rs/serde | Rust | rust cli (lib) | ~43k |
| gin | https://github.com/gin-gonic/gin | Go | go service | ~24k |
| gorilla-mux | https://github.com/gorilla/mux | Go | go service (lib) | ~8k |
| mockito | https://github.com/mockito/mockito | Java | java service (lib) | ~102k |
| junit4 | https://github.com/junit-team/junit4 | Java | java service (lib) | ~45k |
| microservices-demo | https://github.com/GoogleCloudPlatform/microservices-demo | Go/Node/Python/.NET/Java | microservices | ~22k |
| docker-compose | https://github.com/docker/compose | Go | docker deploy | ~57k |
| kind | https://github.com/kubernetes-sigs/kind | Go | k8s deploy | ~27k |

### Category coverage

python service: fastapi, flask, pydantic · ts backend: express, nest, prettier · nextjs
fullstack: shadcn-ui · rust cli: clap, bat, serde · go service: gin, gorilla-mux · java
service: mockito, junit4 · monorepo: svelte · queue+events: celery · db-heavy: sqlalchemy ·
microservices: microservices-demo · docker deploy: docker-compose · k8s deploy: kind

Notes:
- `(lib)` = the repo is a library/toolkit rather than a running service; it fills the
  closest category while giving the atlas a real library-shaped codebase.
- LOC = tracked source lines (`.py .ts .tsx .js .mjs .cjs` for Python/TS repos;
  `.rs`/`.go`/`.java`/`.kt`/`.js`/`.ts`/`.tsx`/`.py` for others), computed with
  `git ls-files -z | xargs -0 wc -l` on the shallow clone; includes tests/docs_src where
  the repo tracks them.
- No repo in the manifest was substituted for a 404; all 20 URLs cloned successfully with
  `--depth 1` (each clone < 20 s on this machine). No local copies were used.

## Re-clone instructions

```bash
# from the repo root (system_ir)
mkdir -p benchmarks/corpus
git clone --depth 1 https://github.com/fastapi/fastapi.git benchmarks/corpus/fastapi
git clone --depth 1 https://github.com/pallets/flask.git benchmarks/corpus/flask
git clone --depth 1 https://github.com/pydantic/pydantic.git benchmarks/corpus/pydantic
git clone --depth 1 https://github.com/sqlalchemy/sqlalchemy.git benchmarks/corpus/sqlalchemy
git clone --depth 1 https://github.com/celery/celery.git benchmarks/corpus/celery
git clone --depth 1 https://github.com/expressjs/express.git benchmarks/corpus/express
git clone --depth 1 https://github.com/nestjs/nest.git benchmarks/corpus/nest
git clone --depth 1 https://github.com/prettier/prettier.git benchmarks/corpus/prettier
git clone --depth 1 https://github.com/sveltejs/svelte.git benchmarks/corpus/svelte
git clone --depth 1 https://github.com/shadcn-ui/ui.git benchmarks/corpus/shadcn-ui
git clone --depth 1 https://github.com/clap-rs/clap.git benchmarks/corpus/clap
git clone --depth 1 https://github.com/sharkdp/bat.git benchmarks/corpus/bat
git clone --depth 1 https://github.com/serde-rs/serde.git benchmarks/corpus/serde
git clone --depth 1 https://github.com/gin-gonic/gin.git benchmarks/corpus/gin
git clone --depth 1 https://github.com/gorilla/mux.git benchmarks/corpus/gorilla-mux
git clone --depth 1 https://github.com/mockito/mockito.git benchmarks/corpus/mockito
git clone --depth 1 https://github.com/junit-team/junit4.git benchmarks/corpus/junit4
git clone --depth 1 https://github.com/GoogleCloudPlatform/microservices-demo.git benchmarks/corpus/microservices-demo
git clone --depth 1 https://github.com/docker/compose.git benchmarks/corpus/docker-compose
git clone --depth 1 https://github.com/kubernetes-sigs/kind.git benchmarks/corpus/kind
```

Ground truth answer keys: `benchmarks/ground-truth/<name>.md` (one doc per repo).

## Holdout corpus (v1)

`benchmarks/holdout/` is the blind holdout set for overfit detection: 20 NEW
repos (5 python / 5 ts / 3 rust / 3 go / 2 java / 2 infra-monorepo), cloned
after the dev-corpus rules were tuned. The atlas has never been fitted
against them. Full manifest + re-clone instructions live in
`benchmarks/holdout/README.md`; answer keys are in
`benchmarks/holdout-ground-truth/<name>.md`.

Run the holdout protocol:

```bash
scc bench atlas --holdout
```

This scores BOTH corpora with the same pipeline, prints the dev-vs-holdout
per-layer recall and the overfit verdict, and writes
`benchmarks/results/holdout-v1.txt` (dev vs holdout per-layer recall, overall,
and the gap). Dev-corpus scoring itself is unchanged by the flag.

## Blind-test corpus (v1)

`benchmarks/blind-test/` is the NEW frozen blind corpus — 20 more repos,
cloned after the validation rules were tuned, that the coding agent NEVER
sees the misses from. Unlike `benchmarks/holdout` (which has been inspected
and tuned against, so it is labelled **validation** in output), the
blind-test corpus is truly blind: `scc bench atlas --blind` prints ONLY
aggregates (overall, per-section means, the validation-vs-blind
generalization gap, precision, density) — no per-repo rows, no missed keys,
no filenames — and `--diagnose` is refused on it. blind-test failures are
never shown to tuning agents.

Category spread (same as dev/holdout): 5 python / 5 ts / 3 rust / 3 go /
2 java / 2 infra-monorepo. No repo overlaps the dev corpus manifest
(fastapi, flask, pydantic, sqlalchemy, celery, express, nest, prettier,
svelte, shadcn-ui, clap, bat, serde, gin, gorilla-mux, mockito, junit4,
microservices-demo, docker-compose, kind) nor the holdout corpus manifest
(django, requests, aiohttp, black, click, react, vue, zod, axios, vitest,
ripgrep, tokio, rayon, cobra, chi, zerolog, netty, guava, helm, kustomize).

## Manifest

| name | URL | language | category | ~LOC (tracked source) |
|---|---|---|---|---|
| httpx | https://github.com/encode/httpx | Python | python http client lib | ~41k |
| jinja2 | https://github.com/pallets/jinja | Python | python template engine | ~30k |
| typer | https://github.com/fastapi/typer | Python | python cli framework | ~54k |
| rich | https://github.com/Textualize/rich | Python | python terminal lib | ~137k |
| redis-py | https://github.com/redis/redis-py | Python | python redis client | ~234k |
| fastify | https://github.com/fastify/fastify | TypeScript/JS | ts backend framework | ~98k |
| hono | https://github.com/honojs/hono | TypeScript | ts web framework | ~97k |
| rxjs | https://github.com/ReactiveX/rxjs | TypeScript | ts reactive lib | ~426k |
| preact | https://github.com/preactjs/preact | JavaScript | ts/js ui framework | ~65k |
| lit | https://github.com/lit/lit | TypeScript | ts web components monorepo | ~298k |
| axum | https://github.com/tokio-rs/axum | Rust | rust web framework | ~64k |
| hyper | https://github.com/hyperium/hyper | Rust | rust http library | ~42k |
| tonic | https://github.com/hyperium/tonic | Rust | rust grpc framework | ~510k |
| fiber | https://github.com/gofiber/fiber | Go | go web framework | ~178k |
| echo | https://github.com/labstack/echo | Go | go web framework | ~46k |
| zap | https://github.com/uber-go/zap | Go | go logging lib | ~27k |
| retrofit | https://github.com/square/retrofit | Java | java http client framework | ~160k |
| gson | https://github.com/google/gson | Java | java json library | ~62k |
| buildx | https://github.com/docker/buildx | Go | infra (docker build toolkit) | ~2444k |
| changesets | https://github.com/changesets/changesets | TypeScript | monorepo release tool | ~56k |

Notes:
- LOC = tracked source lines (all tracked files), computed with
  `git ls-files -z | xargs -0 wc -l` inside the shallow clone; buildx
  counts vendored docs/generated assets.
- All 20 URLs cloned successfully with `--depth 1`; no repo was substituted
  for a 404 and no local copies were used. Clones are gitignored via
  `benchmarks/blind-test/.gitignore` (`*` + `!README.md`); the ground-truth
  docs in `benchmarks/blind-test-ground-truth/` are committed artifacts.

Run the blind protocol:

```bash
scc bench atlas --blind            # aggregates only; writes benchmarks/results/blind-v1.txt
scc bench atlas --blind --json     # same aggregates as JSON
scc bench atlas --blind --diagnose # ERROR: "blind corpus is not diagnosable"
```

Generalization-efficiency tracking (per wave): `benchmarks/compare_waves.py`
reads two result files (holdout-v3.txt / blind-v1.txt) and prints per-layer
deltas + efficiency = validation-delta / development-delta.
