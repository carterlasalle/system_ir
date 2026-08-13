# helm
> https://github.com/helm/helm | Go | infra-monorepo (k8s deploy) | ~128k LOC

## architecture
- cmd/helm — the CLI entrypoint: main + command tree (cmd/helm/helm.go)
- pkg/cmd — CLI command implementations: install, upgrade, rollback, uninstall, list, get, repo, search, pull, push, package, lint, plugin, registry, dependency (pkg/cmd/)
- pkg/action — client actions for the Helm SDK (pkg/action/)
- pkg/chart — chart model: Chart, ChartMetadata, Loader (pkg/chart/)
- pkg/engine — the Go template rendering engine (pkg/engine/)
- pkg/release — release model and lifecycle (pkg/release/)
- pkg/storage — release storage backends (pkg/storage/)
- pkg/repo — chart repository index handling (pkg/repo/)
- pkg/kube — Kubernetes client integration (pkg/kube/)
- pkg/registry — OCI registry client (pkg/registry/)

## entrypoints
- helm — the CLI binary (cmd/helm/helm.go main)
- helm install — install a release
- helm upgrade — upgrade a release
- helm rollback — roll back a release
- helm uninstall — delete a release
- helm list — list releases
- helm get manifest — fetch release manifest
- helm repo add/update — repository management
- helm search repo — search repositories
- helm pull — download a chart
- helm package — package a chart directory
- helm lint — validate a chart
- helm dependency update — fetch chart dependencies
- helm plugin install — install a plugin
- helm create — scaffold a new chart

## behavior
- main -> NewRootCmd -> cmd.Execute — CLI bootstrap (cmd/helm/helm.go)
- install -> action.Install -> release create/install flow (pkg/cmd/install.go + pkg/action/install.go)
- upgrade -> action.Upgrade -> release upgrade flow
- release content -> engine.Render -> rendered manifests (pkg/engine/engine.go)
- kube client apply -> create/update resources — manifest application
- storage driver (secrets/configmap) -> release record persistence
- dependency update -> repo download -> chart unpack (pkg/downloader/)
- registry login/push/pull — OCI registry operations (pkg/registry/)

## state_authority
- action.Install/Upgrade — the action configuration state (pkg/action/)
- settings — global CLI settings (helm.sh/helm/v4/pkg/cli)
- storage driver — release storage backend (pkg/storage/)
- release info — the release record with version/status
- repo file — repositories.yaml config
- KubeClient — Kubernetes client state (pkg/kube/)
- chart cache — downloaded chart storage

## contracts
- helm install <release> <chart> — install command contract
- helm upgrade <release> <chart> — upgrade contract
- helm list -A — list all-namespaces contract
- helm get manifest <release> — manifest retrieval contract
- helm repo add <name> <url> — repo add contract
- helm pull <chart> — pull contract
- helm lint <chart-dir> — lint contract
- --namespace <ns> — namespace flag
- --set key=value — value override flag
- --values <file> — values file flag
- --version <ver> — chart version flag
- chart.yaml apiVersion/name/version — chart metadata contract
- values.yaml — values file contract

## landmarks
- NewRootCmd — root command factory (pkg/cmd/root.go)
- action.Install — install action (pkg/action/install.go)
- action.Upgrade — upgrade action (pkg/action/upgrade.go)
- Engine — template engine (pkg/engine/engine.go)
- Chart — chart model (pkg/chart/chart.go)
- Release — release model (pkg/release/release.go)
- Storage — storage driver abstraction (pkg/storage/storage.go)
- DownloadManager — dependency downloader (pkg/downloader/manager.go)
- registry.Client — OCI client (pkg/registry/client.go)

## tests
- cmd/helm/*_test.go — CLI command tests
- pkg/action/*_test.go — action tests
- pkg/engine/*_test.go — template engine tests
- pkg/chart/*_test.go — chart loading tests
- pkg/repo/*_test.go — repository tests
- pkg/storage/*_test.go — storage tests
