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
