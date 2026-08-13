# SCC Blind-Test Corpus — 20 frozen repos (v1)

The NEW blind corpus for measuring generalization. These 20 repos were
cloned AFTER the validation (`benchmarks/holdout`) rules were tuned and
have never been used in tuning. Unlike the holdout corpus — which has been
inspected and tuned against and is therefore labelled **validation** in
output — the blind-test corpus is truly blind: `scc bench atlas --blind`
prints ONLY aggregates and `--diagnose` is refused, so the coding agent
NEVER sees the misses from these repos.

Each repo lives in `benchmarks/blind-test/<name>/` (shallow clone, `.git`
kept — history is needed by the co-change pipeline). The clones are
gitignored (`benchmarks/blind-test/*` in the root `.gitignore`); this
README and the ground-truth docs in `benchmarks/blind-test-ground-truth/`
are the committed artifacts.

Category spread (mirrors dev + holdout): 5 python / 5 ts / 3 rust / 3 go /
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
  for a 404 and no local copies were used.

## Re-clone instructions

```bash
# from the repo root (system_ir)
mkdir -p benchmarks/blind-test
git clone --depth 1 https://github.com/encode/httpx.git benchmarks/blind-test/httpx
git clone --depth 1 https://github.com/pallets/jinja.git benchmarks/blind-test/jinja2
git clone --depth 1 https://github.com/fastapi/typer.git benchmarks/blind-test/typer
git clone --depth 1 https://github.com/Textualize/rich.git benchmarks/blind-test/rich
git clone --depth 1 https://github.com/redis/redis-py.git benchmarks/blind-test/redis-py
git clone --depth 1 https://github.com/fastify/fastify.git benchmarks/blind-test/fastify
git clone --depth 1 https://github.com/honojs/hono.git benchmarks/blind-test/hono
git clone --depth 1 https://github.com/ReactiveX/rxjs.git benchmarks/blind-test/rxjs
git clone --depth 1 https://github.com/preactjs/preact.git benchmarks/blind-test/preact
git clone --depth 1 https://github.com/lit/lit.git benchmarks/blind-test/lit
git clone --depth 1 https://github.com/tokio-rs/axum.git benchmarks/blind-test/axum
git clone --depth 1 https://github.com/hyperium/hyper.git benchmarks/blind-test/hyper
git clone --depth 1 https://github.com/hyperium/tonic.git benchmarks/blind-test/tonic
git clone --depth 1 https://github.com/gofiber/fiber.git benchmarks/blind-test/fiber
git clone --depth 1 https://github.com/labstack/echo.git benchmarks/blind-test/echo
git clone --depth 1 https://github.com/uber-go/zap.git benchmarks/blind-test/zap
git clone --depth 1 https://github.com/square/retrofit.git benchmarks/blind-test/retrofit
git clone --depth 1 https://github.com/google/gson.git benchmarks/blind-test/gson
git clone --depth 1 https://github.com/docker/buildx.git benchmarks/blind-test/buildx
git clone --depth 1 https://github.com/changesets/changesets.git benchmarks/blind-test/changesets
```

Ground truth answer keys: `benchmarks/blind-test-ground-truth/<name>.md`
(one doc per repo), written by reading the repositories directly — never
through scc. All keys use the same v2 seven-layer ontology as the dev and
holdout corpora (architecture / entrypoints / behavior / state_authority /
contracts / landmarks / tests). The blind property is that the coding agent
never sees the misses, not that the corpus is unexamined.
