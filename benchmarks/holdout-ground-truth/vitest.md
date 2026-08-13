# vitest
> https://github.com/vitest-dev/vitest | TypeScript | ts tool (monorepo) | ~382k LOC

## architecture
- packages/vitest — the main package: CLI, runner, config (packages/vitest/src/)
- packages/vitest/src/node — Node-side orchestration: CLI, project, pool, reporters (packages/vitest/src/node/)
- packages/vitest/src/runtime — worker-side test execution (packages/vitest/src/runtime/)
- packages/expect — expect assertions (packages/expect/src/)
- packages/mocker — vi.mock mocking (packages/mocker/src/)
- packages/spy — vi.spyOn / spy types (packages/spy/src/)
- packages/snapshot — snapshot manager (packages/snapshot/src/)
- packages/ui — web UI for the test browser (packages/ui/)
- packages/coverage-v8 — v8 coverage provider (packages/coverage-v8/src/)

## entrypoints
- vitest — the CLI binary (packages/vitest/src/node/cli.ts)
- vitest run — run-once CLI mode
- vitest watch — watch mode
- test — test declaration (packages/vitest/src/runtime/)
- describe — suite declaration
- it — test declaration alias
- expect — assertion entry
- vi.fn — mock function creation
- vi.mock — module mocking
- config defineConfig — vitest config entry (packages/vitest/src/node/config/)

## behavior
- cli.ts -> startVitest -> createVitest -> runTests — CLI bootstrap (packages/vitest/src/node/cli.ts)
- Vitest.runFiles -> createPool -> runTests in workers — test execution (packages/vitest/src/node/core.ts)
- worker runTests -> describe/it collection -> execution — worker-side run (packages/vitest/src/runtime/run.ts)
- expect -> matchers execution -> assertion result (packages/expect/src/index.ts)
- vi.mock -> hoisted mock registration -> module interception (packages/mocker/src/)
- reporter onTaskUpdate -> terminal output — reporting flow
- watch mode -> watcher -> re-run affected files — watch loop (packages/vitest/src/node/watcher.ts)

## state_authority
- Vitest — the central orchestrator instance (packages/vitest/src/node/core.ts)
- workspace/project — per-project test config state (packages/vitest/src/node/project.ts)
- state — per-file test state collection (packages/vitest/src/node/state.ts)
- config — resolved user config
- mockMap — mock registry (packages/mocker/src/)
- snapshot manager — snapshot file state (packages/snapshot/src/manager.ts)
- ViteNodeServer — vite module graph server

## contracts
- vitest run <pattern> — run command contract
- vitest --watch — watch flag contract
- test('name', fn) — test declaration contract
- describe('suite', () => {...}) — suite contract
- expect(value).toBe(expected) — assertion contract
- expect(x).toThrow() — error assertion contract
- vi.fn(impl) — mock contract
- vi.mock('./module', factory) — module mock contract
- vi.spyOn(obj, 'method') — spy contract
- beforeAll/afterAll hooks — lifecycle contract
- defineConfig({ test: {...} }) — config contract

## landmarks
- startVitest — CLI entry (packages/vitest/src/node/cli.ts)
- Vitest — orchestrator class (packages/vitest/src/node/core.ts)
- runTests — worker run entry (packages/vitest/src/runtime/)
- snapshotManager — snapshot manager (packages/snapshot/src/manager.ts)
- mocker — the mock engine (packages/mocker/src/)
- prettyFormat — formatting (packages/pretty-format/src/)
- CoverageProvider — coverage interface (packages/vitest/src/node/coverage.ts)
- expect — assertion entry (packages/expect/src/index.ts)

## tests
- test/ — the vitest test suite
- test/core/ — core runner tests
- test/cli/ — CLI behavior tests
- test/config/ — config resolution tests
- test/reporters/ — reporter tests
- test/snapshots/ — snapshot tests
- packages/expect/__tests__/ — expect tests
