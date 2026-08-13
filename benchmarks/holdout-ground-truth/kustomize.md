# kustomize
> https://github.com/kubernetes-sigs/kustomize | Go | infra-monorepo (k8s config) | ~239k LOC

## architecture
- kustomize/ — the CLI binary (kustomize/main.go)
- kustomize/commands — CLI command tree (kustomize/commands/)
- api/krusty — the Kustomizer engine entry (api/krusty/kustomizer.go)
- api/internal/target — the kustomization target execution (api/internal/target/)
- api/builtins — built-in generator/transformer plugins (api/builtins/)
- api/resmap — resource map model (api/resmap/)
- api/resource — resource model (api/resource/)
- api/types — kustomization types: Kustomization, GeneratorOptions (api/types/)
- api/filters — resource transformation filters (api/filters/)
- kyaml — the YAML node manipulation library (kyaml/)
- cmd/config — the kyaml-based config commands (cmd/config/)

## entrypoints
- kustomize — the CLI binary (kustomize/main.go)
- kustomize build — the main build command
- kustomize edit — edit kustomization.yaml commands
- kustomize create — scaffold a kustomization.yaml
- kustomize cfg — kyaml config commands (cmd/config/)
- NewDefaultCommand — CLI root construction (kustomize/commands/)
- MakeKustomizer — engine construction (api/krusty/kustomizer.go)
- Kustomizer.Run — run a kustomization (api/krusty/kustomizer.go)
- kustomize build <dir> — build a target directory

## behavior
- main -> NewDefaultCommand().Execute() — CLI bootstrap (kustomize/main.go)
- build -> makeKustomizer -> Kustomizer.Run -> target makeKustomization — build pipeline
- Run -> NewLoader -> NewKustTarget -> makeKustomization — target execution (api/krusty/kustomizer.go)
- makeKustomization -> load kustomization.yaml -> generate resources (api/internal/target/kusttarget.go)
- accumulator merge -> resmap.ResMap accumulation — resource merging
- builtins configmaps/secret generators -> resource creation (api/builtins/)
- kyaml RNode pipeline -> transformer filters -> mutated resources (kyaml/yaml/)
- Run -> accumulate -> ResMap output — result emission

## state_authority
- Kustomizer — the engine state (api/krusty/kustomizer.go)
- Options — engine options: load restrictions, plugin config (api/krusty/options.go)
- KustTarget — per-target state (api/internal/target/kusttarget.go)
- ResMap — the accumulated resource map (api/resmap/resmap.go)
- kustomization.yaml — the source of truth config
- PluginLoader — plugin loading state (api/internal/plugins/loader/)
- filesys.FileSystem — the virtual file system (kyaml/filesys/)

## contracts
- kustomize build <dir> — build command contract
- kustomize build <overlay> --output - — output contract
- kustomization.yaml resources: [...] — resources list contract
- apiVersion: kustomize.config.k8s.io/v1beta1 — kustomization apiVersion contract
- namePrefix: prod- — name prefix transform contract
- commonLabels: {app: web} — common label contract
- configMapGenerator: [{name: x, literals: [...]}] — configmap generator contract
- secretGenerator: [{name: x, files: [...]}] — secret generator contract
- bases: [...] — legacy base reference contract
- patches: [...] — patch contract
- namespace: prod — namespace transform contract
- helmCharts: [{name: chart}] — helm chart inflator contract

## landmarks
- Kustomizer — the engine (api/krusty/kustomizer.go)
- KustTarget — target executor (api/internal/target/kusttarget.go)
- ResMap — resource map type (api/resmap/)
- Resource — single resource type (api/resource/)
- Kustomization — the config type (api/types/kustomization.go)
- ConfigMapGenerator — configmap plugin (api/builtins/configmapgenerator.go)
- SecretGenerator — secret plugin (api/builtins/secretgenerator.go)
- filesys.FileSystem — virtual fs (kyaml/filesys/)
- RNode — kyaml node type (kyaml/yaml/rnode.go)

## tests
- api/krusty/ — engine integration tests
- api/internal/target/ — target tests
- api/builtins/ — plugin tests
- kyaml/ — kyaml tests
- kustomize/commands/ — CLI tests
- api/resmap/ — resmap tests
