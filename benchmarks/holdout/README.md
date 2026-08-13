# SCC Holdout Corpus — 20 blind repos (v1)

Blind holdout corpus for detecting overfitting in the Wave 8 atlas-recall
rules. These 20 repos were cloned AFTER the dev corpus (`benchmarks/corpus`)
rules were tuned; the atlas has never been fitted against them. Each repo
lives in `benchmarks/holdout/<name>/` (shallow clone, `.git` kept). The
clones are gitignored (`benchmarks/holdout/*`); this README and the
ground-truth docs in `benchmarks/holdout-ground-truth/` are the committed
artifacts.

Category spread (mirrors the dev corpus): 5 python / 5 ts / 3 rust / 3 go /
2 java / 2 infra-monorepo. No repo overlaps the dev corpus manifest
(fastapi, flask, pydantic, sqlalchemy, celery, express, nest, prettier,
svelte, shadcn-ui, clap, bat, serde, gin, gorilla-mux, mockito, junit4,
microservices-demo, docker-compose, kind).

## Manifest

| name | URL | language | category | ~LOC (tracked source) |
|---|---|---|---|---|
| django | https://github.com/django/django | Python | python service (framework) | ~1144k |
| requests | https://github.com/psf/requests | Python | python lib | ~31k |
| aiohttp | https://github.com/aio-libs/aiohttp | Python | python service (lib) | ~137k |
| black | https://github.com/psf/black | Python | python tool | ~147k |
| click | https://github.com/pallets/click | Python | python lib | ~39k |
| react | https://github.com/facebook/react | TypeScript/JS | ts frontend framework (monorepo) | ~1178k |
| vue | https://github.com/vuejs/core | TypeScript | ts frontend framework (monorepo) | ~191k |
| zod | https://github.com/colinhacks/zod | TypeScript | ts lib | ~139k |
| axios | https://github.com/axios/axios | TypeScript/JS | ts lib | ~95k |
| vitest | https://github.com/vitest-dev/vitest | TypeScript | ts tool (monorepo) | ~382k |
| ripgrep | https://github.com/BurntSushi/ripgrep | Rust | rust cli | ~77k |
| tokio | https://github.com/tokio-rs/tokio | Rust | rust lib (runtime) | ~191k |
| rayon | https://github.com/rayon-rs/rayon | Rust | rust lib | ~44k |
| cobra | https://github.com/spf13/cobra | Go | go cli lib | ~20k |
| chi | https://github.com/go-chi/chi | Go | go service (router lib) | ~13k |
| zerolog | https://github.com/rs/zerolog | Go | go lib (logging) | ~23k |
| netty | https://github.com/netty/netty | Java | java lib (network) | ~693k |
| guava | https://github.com/google/guava | Java | java lib | ~1045k |
| helm | https://github.com/helm/helm | Go | infra-monorepo (k8s deploy) | ~128k |
| kustomize | https://github.com/kubernetes-sigs/kustomize | Go | infra-monorepo (k8s config) | ~239k |

### Category coverage

python service: django, aiohttp · python lib: requests, click · python tool:
black · ts frontend framework: react, vue · ts lib: zod, axios · ts tool:
vitest · rust cli: ripgrep · rust lib: tokio, rayon · go cli lib: cobra ·
go service: chi · go lib: zerolog · java lib: netty, guava ·
infra-monorepo: helm, kustomize

Notes:
- LOC = tracked source lines (all tracked files), computed with
  `git ls-files -z | xargs -0 wc -l` on the shallow clone; includes
  tests/vendor where the repo tracks them.
- Two deviations from a strict "same distribution" holdout, recorded for
  the overfit verdict:
  1. okhttp (square/okhttp) was cloned first but its 5.x main sources are
     Kotlin; SCC has no Kotlin extractor, so it would have scored ~0 for
     extractor coverage rather than rule generalization. Replaced with
     netty (pure Java).
  2. helm and kustomize (infra-monorepo) are Go-heavy monorepos; there is
     no Go dev-corpus monorepo counterpart (dev used svelte/shadcn-ui for
     monorepo coverage), so this category is inherently new territory.
- No repo in the manifest was substituted for a 404; all 20 URLs cloned
  successfully with `--depth 1` on this machine. No local copies were used.

## Re-clone instructions

```bash
# from the repo root (system_ir)
mkdir -p benchmarks/holdout
git clone --depth 1 https://github.com/django/django.git benchmarks/holdout/django
git clone --depth 1 https://github.com/psf/requests.git benchmarks/holdout/requests
git clone --depth 1 https://github.com/aio-libs/aiohttp.git benchmarks/holdout/aiohttp
git clone --depth 1 https://github.com/psf/black.git benchmarks/holdout/black
git clone --depth 1 https://github.com/pallets/click.git benchmarks/holdout/click
git clone --depth 1 https://github.com/facebook/react.git benchmarks/holdout/react
git clone --depth 1 https://github.com/vuejs/core.git benchmarks/holdout/vue
git clone --depth 1 https://github.com/colinhacks/zod.git benchmarks/holdout/zod
git clone --depth 1 https://github.com/axios/axios.git benchmarks/holdout/axios
git clone --depth 1 https://github.com/vitest-dev/vitest.git benchmarks/holdout/vitest
git clone --depth 1 https://github.com/BurntSushi/ripgrep.git benchmarks/holdout/ripgrep
git clone --depth 1 https://github.com/tokio-rs/tokio.git benchmarks/holdout/tokio
git clone --depth 1 https://github.com/rayon-rs/rayon.git benchmarks/holdout/rayon
git clone --depth 1 https://github.com/spf13/cobra.git benchmarks/holdout/cobra
git clone --depth 1 https://github.com/go-chi/chi.git benchmarks/holdout/chi
git clone --depth 1 https://github.com/rs/zerolog.git benchmarks/holdout/zerolog
git clone --depth 1 https://github.com/netty/netty.git benchmarks/holdout/netty
git clone --depth 1 https://github.com/google/guava.git benchmarks/holdout/guava
git clone --depth 1 https://github.com/helm/helm.git benchmarks/holdout/helm
git clone --depth 1 https://github.com/kubernetes-sigs/kustomize.git benchmarks/holdout/kustomize
```

Ground truth answer keys: `benchmarks/holdout-ground-truth/<name>.md` (one
doc per repo), written by reading the repositories directly — never through
scc. All keys use the same v2 seven-layer ontology as the dev corpus
(architecture / entrypoints / behavior / state_authority / contracts /
landmarks / tests).
