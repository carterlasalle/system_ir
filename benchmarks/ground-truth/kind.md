# kind
> https://github.com/kubernetes-sigs/kind | Go | k8s deploy | ~27k LOC

## architecture
- Provider — pkg/cluster/provider.go public facade for cluster operations (Create/Delete/List)
- Cluster — cluster config type in pkg/apis/config/v1alpha4/types.go (kind: Cluster)
- Node — per-node config type with Role field in pkg/apis/config/v1alpha4/types.go
- create.Cluster — pkg/cluster/internal/create/create.go internal cluster provisioning function
- delete.Cluster — pkg/cluster/internal/delete/delete.go internal teardown function
- nodeimage.Build — pkg/build/nodeimage/build.go node image build entry (Build(options ...Option))
- kubeconfig — pkg/cluster/internal/kubeconfig/kubeconfig.go package with Get/Remove helpers
- app.Main — cmd/kind/app/main.go root command entry called from the stub main.go
- providers.Provider — pkg/cluster/internal/providers/provider.go runtime interface (Provision, ListNodes)
- internalencoding — pkg/internal/apis/config/encoding/load.go config parse/load with v1alpha4 handling

## entrypoints
- `main.go` — stub main wrapping cmd/kind/app app.Main()
- `pkg/cmd/kind/root.go` — root cobra command (Use:   "kind")
- `pkg/cmd/kind/create/create.go` — parent command (Use:   "create")
- `pkg/cmd/kind/create/cluster/createcluster.go` — cluster create command (Use:   "cluster"; "Creates a local Kubernetes cluster")
- `pkg/cmd/kind/delete/delete.go` — parent command (Use:   "delete")
- `pkg/cmd/kind/get/get.go` — parent command (Use:   "get"; clusters, nodes, kubeconfig)
- `func (p *Provider) Create(name string, options ...CreateOption) error` — provider entry for cluster creation
- `func (p *Provider) Delete(name, explicitKubeconfigPath string) error` — provider entry for teardown
- `func (p *Provider) List() ([]string, error)` — provider entry listing existing clusters

## behavior
- `kind create cluster` -> NewProvider -> Provider.Create -> create.Cluster — CLI-to-provider creation flow
- `CreateWithConfigFile` -> internalencoding.Load -> Cluster config — config parsing flow (raw YAML -> typed config)
- `CreateWithWaitForReady` -> wait for control-plane readiness — readiness flow after provisioning
- `Provider.Delete` — teardown flow: delete.Cluster -> p.ListNodes -> remove nodes
- `pkg/cmd/kind/get/kubeconfig` — kubeconfig flow: kubeconfig.Get -> merged kubeconfig output
- `nodeimage.Build` -> docker build of node image — node image build flow
- `docker.NewProvider` — Docker runtime provisioning of node containers (cluster in Docker)
- `CreateWithWaitForReady` — flag-to-option wiring: `wait` flag DurationVar -> flags.Wait -> CreateWithWaitForReady (createcluster.go:115)

## state_authority
- DefaultClusterName — pkg/cluster/constants/constants.go `DefaultClusterName = "kind"` default cluster/context name
- providers.Provider — internal runtime interface owning node lifecycle ops (Provision, ListNodes)
- docker.NewProvider — Docker provider implementation (pkg/cluster/internal/providers/docker)
- podman.NewProvider — Podman provider implementation
- nerdctl.NewProvider — nerdctl provider implementation
- pkg/cluster/internal/kubeconfig — owns kubeconfig file read/write/remove (write_test.go, remove_test.go)
- v1alpha4 — config schema ownership for apiVersion kind.x-k8s.io/v1alpha4 (types.go, yaml.go)

## contracts
- `kind.x-k8s.io/v1alpha4` — config apiVersion contract matched in internal/apis/config/encoding/load.go
- `--config` — cluster create config-file flag (StringVar on flags.Config)
- `--name` — cluster name flag (StringVarP(&flags.Name, "name", "n", ...))
- `--image` — node image override flag (StringVar on flags.ImageName)
- `--retain` — keep nodes on failure flag (BoolVar on flags.Retain)
- `--wait` — wait-for-ready duration flag (DurationVar on flags.Wait)
- `--kubeconfig` — explicit kubeconfig path flag (StringVar on flags.Kubeconfig)
- `ControlPlaneRole NodeRole = "control-plane"` — control-plane role constant in v1alpha4 types.go
- `WorkerRole NodeRole = "worker"` — worker role constant in v1alpha4 types.go

## landmarks
- NewProvider — constructor in pkg/cluster/provider.go taking ProviderOptions (docker/podman/nerdctl runtimes)
- NodeRole — enum type in v1alpha4 types.go: ControlPlaneRole "control-plane", WorkerRole "worker"

## tests
- pkg/internal/apis/config/encoding/load_test.go — config parsing tests
- pkg/internal/apis/config/validate_test.go — config validation tests
- pkg/internal/apis/config/cluster_util_test.go — cluster helper tests
- pkg/cluster/internal/providers/docker/network_test.go — docker network tests
- pkg/cluster/internal/providers/common/namer_test.go — namer tests
- pkg/cluster/internal/kubeconfig/internal/kubeconfig/write_test.go — kubeconfig write tests
- pkg/cluster/internal/kubeconfig/internal/kubeconfig/remove_test.go — kubeconfig remove tests (removes kind entry from kubeconfig)
- pkg/cluster/internal/kubeconfig/internal/kubeconfig/merge_test.go — kubeconfig merge tests (merging multiple contexts)
