# buildx
> https://github.com/docker/buildx | Go | infra: docker build toolkit | ~90k LOC

## architecture
- commands — the CLI commands: build, bake, create, ls, inspect, rm, use (commands/)
- build — the build engine: BuildOptions, runBuild, provenance (build/)
- bake — the bake engine: HCL/JSON bake files (bake/)
- driver — the builder drivers: docker, docker-container, kubernetes, remote (driver/)
- driver/docker — the docker driver (driver/docker/)
- driver/docker-container — the container driver (driver/docker-container/)
- driver/kubernetes — the kubernetes driver (driver/kubernetes/)
- driver/remote — the remote driver (driver/remote/)
- store — the builder instance store: Store, NodeGroup (store/)
- controller — the control API: Controller (controller/)
- monitor — the build monitoring (monitor/)
- policy — build policies (policy/)
- localstate — local build state (localstate/)
- cmd — the main entry (cmd/)
- docs — documentation (docs/)
- tests — the test suite (tests/)

## entrypoints
- buildx build — build an image
- buildx bake — bake from HCL/JSON
- buildx create — create a builder instance
- buildx ls — list builder instances
- buildx inspect — inspect a builder
- buildx rm — remove a builder
- buildx use — select the active builder
- buildx stop — stop a builder
- buildx install — install the build command
- buildx uninstall — uninstall the build command
- buildx diskusage — show builder disk usage
- buildx prune — clean build cache
- buildx version — show version
- buildx debug — debug a build
- buildx dial-stdio — stdio dialing
- buildx imagetools — image tooling
- NewController — the control API entry
- runBuild — the build entry point

## behavior
- buildx create -> driver init -> builder instance (commands/create.go)
- buildx build -> runBuild -> solve -> image (commands/build.go)
- buildx bake -> bake file parse -> build (commands/bake.go)
- buildx ls -> store list -> drivers (commands/ls.go)
- builder solve -> BuildKit session -> output (build/build.go)
- bake parse -> HCL eval -> targets (bake/bake.go)
- docker-container driver -> container launch (driver/docker-container/)
- kubernetes driver -> pod dispatch (driver/kubernetes/)
- store save -> node group persistence (store/store.go)
- prune -> cache GC (commands/prune.go)

## state_authority
- Store — the builder store state (store/store.go)
- NodeGroup — the node group state (store/nodegroup.go)
- Builder — the builder state: nodes, driver (store/)
- Driver — the driver state (driver/driver.go)
- BuildOptions — the build options state (build/build.go)
- bake Target — the bake target state (bake/)
- Controller — the control API state (controller/)
- LocalState — the local build state (localstate/)
- DriverInfo — the driver info state (driver/)

## contracts
- docker buildx build -t image:tag . — build contract
- docker buildx bake --file docker-bake.hcl — bake contract
- docker buildx create --name mybuilder — create contract
- docker buildx ls — ls contract
- docker buildx inspect mybuilder — inspect contract
- docker buildx rm mybuilder — rm contract
- docker buildx use mybuilder — use contract
- docker buildx prune — prune contract
- --platform linux/amd64,linux/arm64 — platform contract
- --push — push contract
- --load — load contract
- --output type=docker — output contract
- --cache-from type=registry,ref=... — cache contract
- --provenance — provenance contract
- --sbom — sbom contract
- --build-arg KEY=value — build arg contract
- --target stage — stage target contract
- docker-container:// — driver endpoint contract
- ssh:// — remote driver contract

## landmarks
- runBuild — the build runner (commands/build.go)
- runBuildWithOptions — the options runner (commands/build.go)
- Controller — the control API (controller/)
- NewController — the controller factory (controller/)
- Store — the store (store/store.go)
- NodeGroup — the node group (store/nodegroup.go)
- Driver — the driver interface (driver/driver.go)
- DriverInfo — the driver info (driver/driver.go)
- BuildOptions — the options (build/build.go)
- Target — the bake target (bake/)
- bakeFile — the bake file (bake/)
- Provenance — the provenance config (build/provenance.go)

## tests
- commands/build_test.go — build command tests
- commands/ls_test.go — ls command tests
- bake/bake_test.go — bake tests
- build/build_test.go — build engine tests
- driver/driver_test.go — driver tests
- store/store_test.go — store tests
- tests/ — integration tests
- tests/integration/ — integration test suite
