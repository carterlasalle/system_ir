# Ground-truth gaps — atlas recall (Wave 8 §57)

Per-repo list of ground-truth keys the startup atlas cannot render, from the 5-repo
QA subset (fastapi, svelte, clap, gin, junit4). One line per gap: `section:key — why`.
Keys already in atlas notation that stay missed are listed as-is; they measure real
extractor/compiler gaps, not string formats. Format-fixed keys are listed per repo at
the end with their atlas-notation form.

## fastapi

- `components:APIWebSocketRoute` — class symbol; atlas has no symbol inventory
- `components:Dependant` — class symbol; no symbol inventory
- `components:ParamTypes` — enum symbol; no symbol inventory
- `components:Security` — class symbol; no symbol inventory
- `components:get_openapi` — function symbol; not in any rendered flow
- `components:BackgroundTasks` — class symbol; no symbol inventory
- `components:HTTPException` — class symbol; no symbol inventory
- `entrypoints:app.get` — instance method; atlas renders route lines (GET /x), not the decorator method
- `entrypoints:app.post` — instance method; atlas renders route lines, not the decorator method
- `entrypoints:include_router` — FastAPI method; not in any rendered flow
- `entrypoints:add_api_route` — FastAPI method; not in any rendered flow
- `entrypoints:websocket` — FastAPI method; not in any rendered flow
- `entrypoints:exception_handler` — FastAPI method; not in any rendered flow
- `entrypoints:openapi` — FastAPI method; not in any rendered flow
- `entrypoints:fastapi = "fastapi.cli:main"` — pyproject console script; extractor reads no [project.scripts] (cli.py defers to fastapi_cli package)
- `flows:get_dependant` — dependency-graph function; flow compiler produced no such flow
- `flows:solve_dependencies` — dependency-resolution function; no flow
- `flows:get_request_handler` — route-wrapper function; no flow
- `flows:request_response` — ASGI wrapper function; no flow
- `flows:get_openapi_path` — OpenAPI function; no flow
- `flows:generate_operation_id` — OpenAPI function; no flow
- `ownership:self.dependency_overrides` — state-holding instance attr; not a data store
- `ownership:self.router` — state-holding instance attr; not a data store
- `ownership:app.state` — state-holding instance attr; not a data store
- `ownership:app.openapi_url` — state-holding instance attr; not a data store
- `contracts:read_items_items__get` — operationId; atlas renders paths, not operation ids
- `contracts:response_model=Item` — response-model contract; atlas renders routes only
- `tests:tests/test_application.py` — test file path; atlas has no test inventory
- `tests:tests/test_dependencies_utils.py` — test file path
- `tests:tests/test_router_include_context.py` — test file path
- `tests:tests/test_openapi_separate_input_output_schemas.py` — test file path
- `tests:tests/test_fastapi_cli.py` — test file path
- `tests:docs_src/app_testing/tutorial003_py310.py` — docs example path
- `tests:tests/test_dependency_overrides.py` — test file path

Format-fixed keys (3 rewritten to atlas notation in `benchmarks/ground-truth/`):
- `contracts:@router.get("/items/{item_id}")`
- `contracts:Header()`
- `entrypoints:@app.get("/items/{item_id}")`

## svelte

- `components:compileModule` — function symbol; not in any rendered flow
- `components:parse` — function symbol; not in any rendered flow
- `components:parseCss` — function symbol; not in any rendered flow
- `components:preprocess` — function symbol; not in any rendered flow
- `components:print` — function symbol; not in any rendered flow
- `components:migrate` — function symbol; not in any rendered flow
- `components:mount` — function symbol; not in any rendered flow
- `components:unmount` — function symbol; not in any rendered flow
- `components:hydrate` — function symbol; not in any rendered flow
- `components:tick` — function symbol; not in any rendered flow
- `components:onMount` — function symbol; not in any rendered flow
- `components:writable` — function symbol; not in any rendered flow
- `entrypoints:svelte/compiler` — package subpath export; no package.json exports layer in atlas
- `entrypoints:svelte/internal` — package subpath export; no exports layer
- `entrypoints:svelte/store` / `svelte/motion` — package subpath exports; no exports layer
- `entrypoints:phases/1-parse` — compiler phase dir; no atlas form
- `entrypoints:phases/2-analyze` — compiler phase dir; no atlas form
- `entrypoints:phases/3-transform` — compiler phase dir; no atlas form
- `flows:parse` — parse flow; flow compiler only built entrypoint-driven flows
- `flows:mount` — mount flow; no flow
- `flows:_mount` — internal mount; no flow
- `flows:migrate` — migrate flow; no flow
- `flows:preprocess` — preprocess flow; no flow
- `flows:flushSync` / `fork` — reactivity internals; no flow
- `ownership:Batch` — state-holding module; not a data store
- `ownership:mounted_components` — state-holding module var; not a data store
- `ownership:get_or_init_context_map` — state-holding module fn; not a data store
- `ownership:active_reaction` — state-holding module var; not a data store
- `ownership:STATE_SYMBOL` — state-holding module const; not a data store
- `ownership:ScopeRoot` — state-holding type; not a data store
- `ownership:css` — per-component analysis state; not a data store
- `contracts:$state` — runes language contract; atlas has no runes/directives layer
- `contracts:$derived` — runes language contract
- `contracts:$effect` — runes language contract
- `contracts:$props` — runes language contract
- `contracts:bind:value` — svelte directive; no directives layer
- `contracts:bind:this` — svelte directive; no directives layer
- `contracts:bind:group` — svelte directive; no directives layer
- `contracts:runes` — compile option; not rendered
- `contracts:css` — compile option; not rendered
- `contracts:mount` — mount options contract; not rendered
- `tests:packages/svelte/tests/compiler-errors` — test dir path
- `tests:packages/svelte/tests/runtime-runes` — test dir path
- `tests:packages/svelte/tests/runtime-legacy` — test dir path
- `tests:packages/svelte/tests/runtime-browser` — test dir path
- `tests:packages/svelte/tests/parser-modern` — test dir path
- `tests:packages/svelte/tests/parser-legacy` — test dir path
- `tests:packages/svelte/tests/snapshot` — test dir path
- `tests:packages/svelte/tests/server-side-rendering` — test dir path
- `tests:packages/svelte/tests/store` — test dir path

Format-fixed keys (2 rewritten to atlas notation in `benchmarks/ground-truth/`):
- `entrypoints:./src/index-client.js`
- `entrypoints:./src/index-server.js`

## clap

- `components:ArgMatches` — struct symbol; no symbol inventory (components are dirs)
- `components:ValueEnum` — derive trait symbol; no symbol inventory
- `components:CommandFactory` — trait symbol; no symbol inventory
- `components:FromArgMatches` — trait symbol; no symbol inventory
- `components:ArgAction` — enum symbol; no symbol inventory
- `components:ErrorKind` — enum symbol; no symbol inventory
- `components:ValueHint` — enum symbol; no symbol inventory
- `components:ColorChoice` — enum symbol; no symbol inventory
- `entrypoints:#[derive(Parser)]` — proc-macro attribute; atlas has no derive-macro layer
- `flows:Command.new` — builder flow; flow compiler produced only entrypoint-driven flows
- `flows:Command.get_matches` — parsing flow; no flow
- `flows:Arg.new` — arg-config flow; no flow
- `flows:Command.subcommand` — subcommand dispatch flow; no flow
- `flows:ArgMatches.get_one` — value-retrieval flow; no flow
- `flows:clap_complete.generate_to` — completion-generation flow; no flow
- `ownership:ArgMatches` — state-holding struct; not a data store
- `ownership:mkeymap` — state-holding struct; not a data store
- `ownership:ArgPredicate` — state-holding struct; not a data store
- `ownership:ValueParser` — state-holding struct; not a data store
- `ownership:StyledStr` — state-holding struct; not a data store
- `contracts:long` — builder API contract; atlas renders CLI flags (--x), not API-level flag names
- `contracts:short('f')` — builder API contract; not rendered
- `tests:clap_builder/src/builder/tests.rs` — test file path
- `tests:examples/` — examples dir path
- `tests:ErrorKind` — doctest symbol; not rendered
- `tests:ArgAction` — doctest symbol; not rendered

Format-fixed keys (13 rewritten to atlas notation in `benchmarks/ground-truth/`):
- `contracts:Arg::num_args`
- `contracts:Arg::required`
- `contracts:Command::arg_required_else_help`
- `contracts:Command::bin_name`
- `contracts:ErrorKind::DisplayHelp`
- `contracts:ErrorKind::InvalidValue`
- `contracts:ErrorKind::UnknownArgument`
- `entrypoints:Command::arg`
- `entrypoints:Command::get_matches`
- `entrypoints:Command::get_matches_from`
- `entrypoints:Command::new`
- `entrypoints:Command::subcommand`
- `entrypoints:clap_complete::generate`

## gin

- `components:HandlerFunc` — func-type symbol; no symbol inventory
- `components:RouteInfo` — struct symbol; no symbol inventory
- `components:methodTree` — unexported struct symbol; no symbol inventory
- `entrypoints:gin.New()` — package constructor; go extractor names it `New`, no flow renders it
- `entrypoints:gin.Default()` — package constructor; no flow renders it
- `entrypoints:Engine.ServeHTTP` — http.Handler method; not in any rendered flow
- `entrypoints:RouterGroup.GET` — route-registration method; not in any rendered flow
- `entrypoints:RouterGroup.POST` — route-registration method; not in any rendered flow
- `entrypoints:RouterGroup.Group` — subgroup method; not in any rendered flow
- `entrypoints:RouterGroup.Use` — middleware method; not in any rendered flow
- `flows:gin.Default` — app-setup flow; no flow
- `flows:Context.JSON` — response flow; no flow
- `flows:RouterGroup.handle` — registration flow; no flow
- `flows:Context.Next` — middleware-chain flow; no flow
- `ownership:engine.trees` — state-holding field; not a data store
- `ownership:engine.RouterGroup` — state-holding field; not a data store
- `ownership:engine.noRoute` — state-holding field; not a data store
- `ownership:RouterGroup.BasePath` — state-holding field; not a data store
- `contracts:router.GET("/ping"` — go extractor detects no routes; atlas renders only extracted route entities
- `contracts:GET /test` — route registered in gin_test.go; no route extraction for go
- `contracts:GET /:count` — route in gin_test.go; no route extraction for go
- `contracts:GET /v1/:path` — route in gin_test.go; no route extraction for go
- `contracts:GET /test/:param` — route in gin_test.go; no route extraction for go
- `contracts:POST /` — route; no route extraction for go
- `contracts:Engine.NoRoute` — API method; not rendered
- `contracts:Context.Param` — API method; not rendered
- `contracts:Context.ShouldBindJSON` — API method; not rendered
- `tests:gin_test.go` — test file path
- `tests:gin_integration_test.go` — test file path
- `tests:benchmarks_test.go` — test file path
- `tests:context_test.go` — test file path
- `tests:auth_test.go` — test file path
- `tests:errors_test.go` — test file path
- `tests:router.Group("/users")` — test-group route form; no route extraction

Format-fixed keys (0 rewritten to atlas notation in `benchmarks/ground-truth/`):

## junit4

- `components:Assert` — class symbol; java disabled
- `components:JUnitCore` — class symbol; java disabled
- `components:Result` — class symbol; java disabled
- `components:Runner` — class symbol; java disabled
- `components:Description` — class symbol; java disabled
- `components:ParentRunner` — class symbol; java disabled
- `components:BlockJUnit4ClassRunner` — class symbol; java disabled
- `components:Suite` — class symbol; java disabled
- `components:Parameterized` — class symbol; java disabled
- `components:Before` — annotation symbol; java disabled
- `components:TestRule` — interface symbol; java disabled
- `components:JUnit38ClassRunner` — class symbol; java disabled
- `components:RunNotifier` — class symbol; java disabled
- `entrypoints:JUnitCore.main` — main method entrypoint; java disabled
- `entrypoints:JUnitCore.run` — facade method; java disabled
- `entrypoints:runClasses(Class<?>... classes)` — static convenience method; java disabled
- `entrypoints:JUnitCommandLineParseResult` — CLI-parsing class; java disabled
- `entrypoints:@RunWith` — annotation; no annotations layer
- `entrypoints:Request.classes` — static method; java disabled
- `entrypoints:@Test` — annotation; no annotations layer
- `flows:JUnitCore.run` -> Request.classes -> Computer -> Runner` — runner-assembly flow; java disabled, no flows
- `flows:BlockJUnit4ClassRunner` -> MethodRoadie -> RunRules -> before/test/after` — per-method flow; no flows
- `flows:ParentRunner.run` -> runChildren` — recursive runner flow; no flows
- `flows:RunNotifier.fireTestStarted` -> listeners` — notification flow; no flows
- `flows:--filter` -> FilterFactories -> Filter` — CLI filtering flow; no flows
- `flows:assertThrows` -> ThrowingRunnable.run() inside try/catch` — assertion flow; no flows
- `flows:@RunWith(Parameterized.class)` -> Parameterized -> per-parameter-set child runners` — parameterized flow; no flows
- `ownership:Result` — state-holding class; not a data store
- `ownership:Description` — state-holding class; not a data store
- `ownership:TestClass` — state-holding class; not a data store
- `ownership:TestMethod` — state-holding class; not a data store
- `ownership:RuleContainer` — state-holding class; not a data store
- `ownership:RunNotifier` — state-holding class; not a data store
- `contracts:@Test` — annotation contract; no annotations layer
- `contracts:timeout()` — annotation attribute; no annotations layer
- `contracts:assertEquals(Object expected, Object actual)` — assertion API; java disabled
- `contracts:assertThrows` — assertion API; java disabled
- `contracts:@Before` — annotation; no annotations layer
- `contracts:@BeforeClass` — annotation; no annotations layer
- `contracts:@Rule` — annotation; no annotations layer
- `contracts:@Ignore` — annotation; no annotations layer
- `contracts:@FixMethodOrder(MethodSorters.NAME_ASCENDING)` — annotation; no annotations layer
- `tests:AssertionTest` — test class name; no test inventory
- `tests:JUnitCoreReturnsCorrectExitCodeTest` — test class name; no test inventory
- `tests:CommandLineTest` — test class name; no test inventory
- `tests:TimeoutTest` — test class name; no test inventory
- `tests:ExpectedTest` — test class name; no test inventory
- `tests:AllTests` — test class name; no test inventory
- `tests:AllCoreTests` — test class name; no test inventory

Format-fixed keys (0 rewritten to atlas notation in `benchmarks/ground-truth/`):
