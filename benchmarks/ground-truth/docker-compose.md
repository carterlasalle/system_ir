# docker-compose
> https://github.com/docker/compose | Go | docker deploy | ~57k LOC

## architecture
- composeService — pkg/compose/compose.go central implementation of the compose API (holds dockerCli, clock, prompt)
- api.Compose — pkg/api/api.go interface of all compose operations (Build/Pull/Create/Up/Down/Ps/Exec/Logs)
- types.Project — compose-go project model (github.com/compose-spec/compose-go/v2/types) passed to every operation
- RootCommand — cmd/compose/compose.go builds the root cobra command tree for `docker compose`
- pluginMain — cmd/main.go Docker CLI plugin entrypoint (plugin.Run wrapping RootCommand)
- EventProcessor — pkg/api/event.go interface notified of compose operations (Start/On/Err)

## entrypoints
- `upCommand` — cobra command `up [OPTIONS] [SERVICE...]` in cmd/compose/up.go
- `downCommand` — cobra command `down [OPTIONS] [SERVICES]` in cmd/compose/down.go
- `psCmd` — cobra command `ps [OPTIONS] [SERVICE...]` in cmd/compose/ps.go
- `startCommand` — cobra command `start [SERVICE...]` in cmd/compose/start.go
- `stopCommand` — cobra command `stop [OPTIONS] [SERVICE...]` in cmd/compose/stop.go
- `restartCmd` — cobra command `restart [OPTIONS] [SERVICE...]` in cmd/compose/restart.go
- `pullCommand` — cobra command `pull [OPTIONS] [SERVICE...]` in cmd/compose/pull.go
- `logsCmd` — cobra command `logs [OPTIONS] [SERVICE...]` in cmd/compose/logs.go
- `exec` — cobra command `exec [OPTIONS] SERVICE COMMAND [ARGS...]` in cmd/compose/exec.go
- `build` — cobra command `build [OPTIONS] [SERVICE...]` in cmd/compose/build.go
- `Up(ctx context.Context, project *types.Project, options api.UpOptions) error` — composeService.Up in pkg/compose/up.go, the core bring-up operation
- `Down(ctx context.Context, projectName string, options api.DownOptions) error` — composeService.Down in pkg/compose/down.go, the teardown operation

## behavior
- `up` -> composeService.Up -> create -> start — main bring-up flow (up.go calls s.create then s.start)
- `down` -> composeService.Down -> remove containers/networks — teardown flow in down.go
- `create` -> createNetwork/createVolume/container creation — create.go provisioning flow
- `runUp` -> backend.Up with Create/Build options — cmd-level orchestration in up.go
- `ps` -> composeService.Ps -> []api.ContainerSummary — listing flow with oneOffExclude filtering
- `pull` -> composeService.Pull -> image pulls — pull flow in pull.go
- `logs` -> composeService.Logs -> LogConsumer callbacks — log streaming flow
- `exec` -> composeService.Exec -> run command in container — exec flow

## state_authority
- composeService — owns dockerCli, clock, prompt, events, and container state for all compose operations
- types.Project — owns the service/network/volume model (ServiceNames(), WithSelectedServices())
- api.UpOptions — options struct in pkg/api owning up flags (Detach, noStart, cascadeStop, etc.)
- api.DownOptions — options struct owning teardown flags (volumes, removeOrphans)
- ProjectOptions — owns cmd-level project loading options shared by commands
- EventProcessor — event bus ownership for progress reporting across operations
- executor — pkg/compose/executor.go applies per-container operations against the engine

## contracts
- `up [OPTIONS] [SERVICE...]` — up command contract (Use string in cmd/compose/up.go)
- `down [OPTIONS] [SERVICES]` — down command contract (Use string in cmd/compose/down.go)
- `ps [OPTIONS] [SERVICE...]` — ps command contract (Use string in cmd/compose/ps.go)
- `exec [OPTIONS] SERVICE COMMAND [ARGS...]` — exec command contract (Use string in cmd/compose/exec.go)
- `build [OPTIONS] [SERVICE...]` — build command contract (Use string in cmd/compose/build.go)
- `pull [OPTIONS] [SERVICE...]` — pull command contract (Use string in cmd/compose/pull.go)
- `restart [OPTIONS] [SERVICE...]` — restart command contract (Use string in cmd/compose/restart.go)
- `logs [OPTIONS] [SERVICE...]` — logs command contract (Use string in cmd/compose/logs.go)
- `--detach` — up flag "Detached mode: Run containers in the background" (BoolVarP with -d)
- `--force-recreate` — up/create flag to recreate containers even if unchanged
- `--remove-orphans` — up flag to remove containers not defined in the Compose file

## landmarks
- NewComposeService — constructor in pkg/compose/compose.go wiring dockerCli, options, and events into a composeService
- runUp — cmd/compose/up.go orchestration of create+start for the `up` command
- Run — pkg/compose/progress.go helper executing an operation with Start/Done events on the event bus
- ProjectOptions — cmd/compose/compose.go options struct owning project loading state (shared by up/down/etc.)

## tests
- pkg/compose/create_test.go — container-creation unit tests
- pkg/compose/down_test.go — teardown unit tests
- pkg/compose/loader_test.go — compose-file loading tests
- pkg/compose/executor_test.go — operation application tests
- cmd/compose/up_test.go — up command tests
- cmd/compose/compose_test.go — root command tests
- pkg/e2e — end-to-end suite (compose_up_test.go, compose_run_test.go, compose_exec_test.go) driving real daemons
