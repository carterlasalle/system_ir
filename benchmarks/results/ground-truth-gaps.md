# Ground-truth gaps — atlas recall (Wave 8 §57)

Per-repo list of ground-truth keys the startup atlas cannot render, from the
full 20-repo corpus QA pass (first 5 repos in the subset pass, remaining 15 in
this pass). One line per gap: `section:key` — why. Keys already in atlas
notation that stay missed are listed as-is; they measure real
extractor/compiler gaps, not string formats. Format-fixed keys are listed per
repo at the end with their atlas-notation form.

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

## bat

- `components:Controller` — struct/trait/enum symbol; atlas has no symbol inventory (components are dirs)
- `components:PrettyPrinter` — struct/trait/enum symbol; atlas has no symbol inventory (components are dirs)
- `components:Printer` — struct/trait/enum symbol; atlas has no symbol inventory (components are dirs)
- `components:SimplePrinter` — struct/trait/enum symbol; atlas has no symbol inventory (components are dirs)
- `components:InteractivePrinter` — struct/trait/enum symbol; atlas has no symbol inventory (components are dirs)
- `components:SyntaxMapping` — struct/trait/enum symbol; atlas has no symbol inventory (components are dirs)
- `components:PagingMode` — struct/trait/enum symbol; atlas has no symbol inventory (components are dirs)
- `components:OutputHandle` — struct/trait/enum symbol; atlas has no symbol inventory (components are dirs)
- `entrypoints:Controller::run` — method symbol; not in any rendered flow
- `entrypoints:Controller::run_with_error_handler` — method symbol; not in any rendered flow
- `entrypoints:bat cache` — subcommand registered via clap builder `Command::new("cache")`; now renders as `build_app [cli-subcommand]`, but the `bat cache` invocation string does not
- `flows:App.new` — flow; flow compiler produced only entrypoint-driven flows
- `flows:App.inputs` — flow; flow compiler produced only entrypoint-driven flows
- `flows:Controller.run` — flow; flow compiler produced only entrypoint-driven flows
- `flows:Assets.from_cache` — flow; flow compiler produced only entrypoint-driven flows
- `flows:SyntaxMapping.get_syntax_for` — flow; flow compiler produced only entrypoint-driven flows
- `ownership:InputDescription` — state-holding struct; not a data store
- `ownership:PrettyPrinter` — state-holding struct; not a data store
- `ownership:Controller` — state-holding struct; not a data store
- `tests:tests/integration_tests.rs` — test file/dir path; atlas has no test inventory
- `tests:tests/snapshot_tests.rs` + `tests/snapshots/` — test file/dir path; atlas has no test inventory
- `tests:tests/test_pretty_printer.rs` — test file/dir path; atlas has no test inventory
- `tests:tests/syntax-tests` — test file/dir path; atlas has no test inventory
- `tests:tests/benchmarks` — test file/dir path; atlas has no test inventory
- `tests:src/syntax_mapping.rs` `mod tests` (line 190)` — test file/dir path; atlas has no test inventory
- `tests:tests/github-actions.rs` — test file/dir path; atlas has no test inventory

Format-fixed keys (1 rewritten to atlas notation in `benchmarks/ground-truth/`):
- `entrypoints:fn main` -> `main`

## celery

- `components:chain` — class/function symbol; no symbol inventory
- `components:AsyncResult` — class/function symbol; no symbol inventory
- `components:crontab` — class/function symbol; no symbol inventory
- `components:RPCBackend` — class/function symbol; no symbol inventory
- `components:BaseKeyValueStoreBackend` — class/function symbol; no symbol inventory
- `entrypoints:@app.task` — decorator; atlas renders routes, not decorator methods
- `entrypoints:@shared_task` — decorator; atlas renders routes, not decorator methods
- `entrypoints:app.send_task` — instance method; not in any rendered flow
- `entrypoints:task.delay` — instance method; not in any rendered flow
- `entrypoints:task.apply_async` — instance method; not in any rendered flow
- `entrypoints:app.worker_main` — instance method; not in any rendered flow
- `entrypoints:app.start` — instance method; not in any rendered flow
- `entrypoints:celery -A proj worker` — CLI invocation string; no atlas form
- `flows:chord header -> add_unlock_chord_task` — flow; flow compiler produced only route/patch-driven flows
- `flows:worker consumer -> task.run` — flow; flow compiler produced only route/patch-driven flows
- `ownership:app.conf` — config key/state holder; not a data store
- `ownership:broker_url` — config key/state holder; not a data store
- `ownership:result_backend` — config key/state holder; not a data store
- `ownership:task_keyprefix = 'celery-task-meta-'` — config key/state holder; not a data store
- `ownership:celery/worker/state.py` — config key/state holder; not a data store
- `ownership:PersistentScheduler` — config key/state holder; not a data store
- `contracts:--broker` — CLI option; celery click options not extracted
- `contracts:--result-backend` — CLI option; celery click options not extracted
- `contracts:-Q` — CLI option; celery click options not extracted
- `contracts:--concurrency` — CLI option; celery click options not extracted
- `contracts:--pool` — CLI option; celery click options not extracted
- `contracts:broker='amqp://guest@localhost//'` — config/URL contract; not rendered
- `contracts:redis://localhost:6379/0` — config/URL contract; not rendered
- `contracts:name='celery.accumulate'` — config/URL contract; not rendered
- `tests:t/integration/` — test dir/file path; no test inventory
- `tests:t/unit/tasks/test_canvas.py` — test dir/file path; no test inventory

Format-fixed keys (3 rewritten to atlas notation in `benchmarks/ground-truth/`):
- `flows:apply_async -> send_task -> broker publish` -> `apply_async`
- `flows:group -> GroupResult` -> `group`
- `flows:Scheduler -> ScheduleEntry -> apply_async` -> `Scheduler`

## docker-compose

- `components:NewComposeService` — struct/interface/constructor symbol; no symbol inventory
- `components:api.Compose` — struct/interface/constructor symbol; no symbol inventory
- `components:types.Project` — struct/interface/constructor symbol; no symbol inventory
- `components:RootCommand` — struct/interface/constructor symbol; no symbol inventory
- `components:runUp` — struct/interface/constructor symbol; no symbol inventory
- `components:EventProcessor` — struct/interface/constructor symbol; no symbol inventory
- `components:ProjectOptions` — struct/interface/constructor symbol; no symbol inventory
- `entrypoints:upCommand` — cobra command var; go extractor emits subcommands by Use name, not the var
- `entrypoints:downCommand` — cobra command var; go extractor emits subcommands by Use name, not the var
- `entrypoints:psCmd` — cobra command var; go extractor emits subcommands by Use name, not the var
- `entrypoints:startCommand` — cobra command var; go extractor emits subcommands by Use name, not the var
- `entrypoints:stopCommand` — cobra command var; go extractor emits subcommands by Use name, not the var
- `entrypoints:restartCmd` — cobra command var; go extractor emits subcommands by Use name, not the var
- `entrypoints:pullCommand` — cobra command var; go extractor emits subcommands by Use name, not the var
- `entrypoints:logsCmd` — cobra command var; go extractor emits subcommands by Use name, not the var
- `entrypoints:Up(ctx context.Context, project *types.Project, options api.UpOptions) error` — method signature; not in any rendered flow
- `entrypoints:Down(ctx context.Context, projectName string, options api.DownOptions) error` — method signature; not in any rendered flow
- `flows:up` -> composeService.Up -> create -> start` — flow; flow compiler produced only entrypoint-driven flows
- `flows:down` -> composeService.Down -> remove containers/networks` — flow; flow compiler produced only entrypoint-driven flows
- `flows:create` -> createNetwork/createVolume/container creation` — flow; flow compiler produced only entrypoint-driven flows
- `flows:runUp` -> backend.Up with Create/Build options` — flow; flow compiler produced only entrypoint-driven flows
- `flows:ps` -> composeService.Ps -> []api.ContainerSummary` — flow; flow compiler produced only entrypoint-driven flows
- `flows:pull` -> composeService.Pull -> image pulls` — flow; flow compiler produced only entrypoint-driven flows
- `flows:logs` -> composeService.Logs -> LogConsumer callbacks` — flow; flow compiler produced only entrypoint-driven flows
- `flows:exec` -> composeService.Exec -> run command in container` — flow; flow compiler produced only entrypoint-driven flows
- `ownership:types.Project` — options/state struct; not a data store
- `ownership:api.UpOptions` — options/state struct; not a data store
- `ownership:api.DownOptions` — options/state struct; not a data store
- `ownership:ProjectOptions` — options/state struct; not a data store
- `ownership:EventProcessor` — options/state struct; not a data store
- `ownership:executor` — options/state struct; not a data store
- `contracts:up [OPTIONS] [SERVICE...]` — cobra Use contract string; not rendered
- `contracts:down [OPTIONS] [SERVICES]` — cobra Use contract string; not rendered
- `contracts:ps [OPTIONS] [SERVICE...]` — cobra Use contract string; not rendered
- `contracts:exec [OPTIONS] SERVICE COMMAND [ARGS...]` — cobra Use contract string; not rendered
- `contracts:build [OPTIONS] [SERVICE...]` — cobra Use contract string; not rendered
- `contracts:pull [OPTIONS] [SERVICE...]` — cobra Use contract string; not rendered
- `contracts:restart [OPTIONS] [SERVICE...]` — cobra Use contract string; not rendered
- `contracts:logs [OPTIONS] [SERVICE...]` — cobra Use contract string; not rendered
- `contracts:--detach` — CLI flag; cobra flags on command vars not extracted
- `contracts:--force-recreate` — CLI flag; cobra flags on command vars not extracted
- `contracts:--remove-orphans` — CLI flag; cobra flags on command vars not extracted
- `tests:pkg/compose/create_test.go` — test file/dir path; no test inventory
- `tests:pkg/compose/down_test.go` — test file/dir path; no test inventory
- `tests:pkg/compose/loader_test.go` — test file/dir path; no test inventory
- `tests:pkg/compose/executor_test.go` — test file/dir path; no test inventory
- `tests:cmd/compose/up_test.go` — test file/dir path; no test inventory
- `tests:cmd/compose/compose_test.go` — test file/dir path; no test inventory
- `tests:pkg/e2e` — test file/dir path; no test inventory

Format-fixed keys (0 rewritten to atlas notation in `benchmarks/ground-truth/`):


## express

- `components:createApplication` — module-level JS symbol; no symbol inventory
- `components:urlencoded` — module-level JS symbol; no symbol inventory
- `components:methods` — module-level JS symbol; no symbol inventory
- `components:app.request` — module-level JS symbol; no symbol inventory
- `entrypoints:require('express')` — package entry expression; not rendered
- `entrypoints:app.listen` — instance method; not in any rendered flow
- `entrypoints:app.handle` — instance method; not in any rendered flow
- `entrypoints:app.init` — instance method; not in any rendered flow
- `entrypoints:app.engine` — instance method; not in any rendered flow
- `entrypoints:app.param` — instance method; not in any rendered flow
- `entrypoints:exports.application` — instance method; not in any rendered flow
- `flows:app.route` -> `router.route` — flow; flow compiler produced only route/entrypoint-driven flows
- `flows:methods.forEach` -> `app.get` — flow; flow compiler produced only route/entrypoint-driven flows
- `flows:app.listen` -> `http.createServer(this)` -> `app.handle` — flow; flow compiler produced only route/entrypoint-driven flows
- `flows:app.render` -> `tryRender` -> `View.prototype.render` — flow; flow compiler produced only route/entrypoint-driven flows
- `flows:res.sendFile` -> `sendfile` helper` — flow; flow compiler produced only route/entrypoint-driven flows
- `flows:res.redirect` — flow; flow compiler produced only route/entrypoint-driven flows
- `flows:res.json` — flow; flow compiler produced only route/entrypoint-driven flows
- `ownership:this.router` — instance field; not a data store
- `ownership:this.settings` — instance field; not a data store
- `ownership:this.engines` — instance field; not a data store
- `ownership:this.cache` — instance field; not a data store
- `ownership:res.locals` — instance field; not a data store
- `ownership:req.app` — instance field; not a data store
- `ownership:trustProxyDefaultSymbol` — instance field; not a data store
- `contracts:case sensitive routing` — setting/API contract; not rendered
- `contracts:view engine` — setting/API contract; not rendered
- `contracts:query parser` — setting/API contract; not rendered
- `contracts:res.status(code)` — setting/API contract; not rendered
- `tests:test/app.param.js` — test file path; no test inventory
- `tests:test/app.head.js` — test file path; no test inventory
- `tests:test/app.options.js` — test file path; no test inventory
- `tests:test/res.json.js` — test file path; no test inventory
- `tests:test/req.ip.js` — test file path; no test inventory
- `tests:test/req.route.js` — test file path; no test inventory
- `tests:test/utils.js` — test file path; no test inventory

Format-fixed keys (3 rewritten to atlas notation in `benchmarks/ground-truth/`):
- `contracts:app.get('/tobi'` -> `GET /tobi`
- `contracts:app.get('/post/:id'` -> `GET /post/:id`
- `contracts:app.get('/user/:id{/:op}'` -> `GET /user/:id{/:op}`

## flask

- `components:MethodView` — class symbol; no symbol inventory
- `components:SessionInterface` — class symbol; no symbol inventory
- `components:SecureCookieSessionInterface` — class symbol; no symbol inventory
- `components:Config` — class symbol; no symbol inventory
- `components:Environment` — class symbol; no symbol inventory
- `components:DispatchingJinjaLoader` — class symbol; no symbol inventory
- `components:FlaskGroup` — class symbol; no symbol inventory
- `entrypoints:app.run` — method/function; not in any rendered flow
- `entrypoints:@app.route` — decorator; atlas renders route lines (GET /x), not the decorator method
- `entrypoints:@app.get` — decorator; atlas renders route lines (GET /x), not the decorator method
- `entrypoints:@app.post` — decorator; atlas renders route lines (GET /x), not the decorator method
- `entrypoints:register_blueprint` — method/function; not in any rendered flow
- `entrypoints:add_url_rule` — method/function; not in any rendered flow
- `entrypoints:flask = "flask.cli:main"` — pyproject [project.scripts] console script; extractor reads no scripts
- `entrypoints:flask run` — method/function; not in any rendered flow
- `entrypoints:locate_app` — method/function; not in any rendered flow
- `flows:full_dispatch_request` — request-pipeline function; no flow
- `flows:dispatch_request` — request-pipeline function; no flow
- `flows:preprocess_request` — request-pipeline function; no flow
- `flows:process_response` — request-pipeline function; no flow
- `flows:url_for` — request-pipeline function; no flow
- `flows:find_best_app` — request-pipeline function; no flow
- `ownership:app.view_functions` — instance attr/state; not a data store
- `ownership:self.url_map` — instance attr/state; not a data store
- `ownership:session_interface` — instance attr/state; not a data store
- `ownership:jinja_env` — instance attr/state; not a data store
- `ownership:app.config` — instance attr/state; not a data store
- `ownership:flask.session` — instance attr/state; not a data store
- `contracts:--host` — CLI option/command; click options not extracted
- `contracts:flask routes` — CLI option/command; click options not extracted
- `tests:tests/test_basic.py` — test file path; no test inventory
- `tests:tests/test_blueprints.py` — test file path; no test inventory
- `tests:tests/test_cli.py` — test file path; no test inventory
- `tests:tests/test_templating.py` — test file path; no test inventory
- `tests:tests/test_user_error_handler.py` — test file path; no test inventory
- `tests:tests/test_session_interface.py` — test file path; no test inventory
- `tests:tests/test_views.py` — test file path; no test inventory

Format-fixed keys (1 rewritten to atlas notation in `benchmarks/ground-truth/`):
- `contracts:@app.route("/", methods=["GET", "POST"])` -> `GET /`

## gorilla-mux

- `components:MatcherFunc` — func-type symbol; no symbol inventory
- `components:MiddlewareFunc` — func-type symbol; no symbol inventory
- `entrypoints:Router.HandleFunc` — Router method; not in any rendered flow
- `entrypoints:Router.Handle` — Router method; not in any rendered flow
- `entrypoints:Router.NewRoute` — Router method; not in any rendered flow
- `entrypoints:Router.Match` — Router method; not in any rendered flow
- `flows:PathPrefix` — registration flow; no flow
- `flows:Router.Use` — registration flow; no flow
- `ownership:Router.routes` — unexported field; not a data store
- `ownership:Router.namedRoutes` — unexported field; not a data store
- `ownership:varsKey` — unexported field; not a data store
- `ownership:Router.routeConf` — unexported field; not a data store
- `contracts:r.HandleFunc("/api", emptyHandler).Methods("POST")` — route-builder API call; go extractor detects no gorilla routes
- `contracts:Queries("time", "{time:[0-9]+}")` — route-builder API call; go extractor detects no gorilla routes
- `contracts:Queries("foo", "{foo:[0-9]+}")` — route-builder API call; go extractor detects no gorilla routes
- `contracts:PathPrefix("/sub/").Subrouter()` — route-builder API call; go extractor detects no gorilla routes
- `contracts:Host("{subdomain}.domain.com")` — route-builder API call; go extractor detects no gorilla routes
- `contracts:r.HandleFunc("/", func1).Name("func1")` — route-builder API call; go extractor detects no gorilla routes
- `contracts:{name:pattern}` — route-builder API call; go extractor detects no gorilla routes
- `tests:mux_test.go` — test file path; no test inventory
- `tests:route_test.go` — test file path; no test inventory
- `tests:regexp_test.go` — test file path; no test inventory
- `tests:example_route_test.go` — test file path; no test inventory
- `tests:example_authentication_middleware_test.go` — test file path; no test inventory
- `tests:example_cors_method_middleware_test.go` — test file path; no test inventory
- `tests:mux_httpserver_test.go` — test file path; no test inventory
- `tests:bench_test.go` — test file path; no test inventory

Format-fixed keys (0 rewritten to atlas notation in `benchmarks/ground-truth/`):


## kind

- `components:Provider` — constructor/type symbol; no symbol inventory
- `components:NewProvider` — constructor/type symbol; no symbol inventory
- `components:NodeRole` — constructor/type symbol; no symbol inventory
- `components:create.Cluster` — constructor/type symbol; no symbol inventory
- `components:delete.Cluster` — constructor/type symbol; no symbol inventory
- `components:nodeimage.Build` — constructor/type symbol; no symbol inventory
- `components:app.Main` — constructor/type symbol; no symbol inventory
- `components:providers.Provider` — constructor/type symbol; no symbol inventory
- `components:internalencoding` — constructor/type symbol; no symbol inventory
- `entrypoints:main.go` — entry file path; atlas renders symbols, not entry files
- `entrypoints:pkg/cmd/kind/root.go` — entry file path; atlas renders symbols, not entry files
- `entrypoints:pkg/cmd/kind/create/create.go` — entry file path; atlas renders symbols, not entry files
- `entrypoints:pkg/cmd/kind/create/cluster/createcluster.go` — entry file path; atlas renders symbols, not entry files
- `entrypoints:pkg/cmd/kind/delete/delete.go` — entry file path; atlas renders symbols, not entry files
- `entrypoints:pkg/cmd/kind/get/get.go` — entry file path; atlas renders symbols, not entry files
- `entrypoints:func (p *Provider) Create(name string, options ...CreateOption) error` — method signature; not in any rendered flow
- `entrypoints:func (p *Provider) Delete(name, explicitKubeconfigPath string) error` — method signature; not in any rendered flow
- `entrypoints:func (p *Provider) List() ([]string, error)` — method signature; not in any rendered flow
- `flows:kind create cluster` -> NewProvider -> Provider.Create -> create.Cluster` — flow; flow compiler produced only entrypoint-driven flows
- `flows:CreateWithConfigFile` -> internalencoding.Load -> Cluster config` — flow; flow compiler produced only entrypoint-driven flows
- `flows:CreateWithWaitForReady` -> wait for control-plane readiness` — flow; flow compiler produced only entrypoint-driven flows
- `flows:Provider.Delete` — flow; flow compiler produced only entrypoint-driven flows
- `flows:pkg/cmd/kind/get/kubeconfig` — flow; flow compiler produced only entrypoint-driven flows
- `flows:nodeimage.Build` -> docker build of node image` — flow; flow compiler produced only entrypoint-driven flows
- `flows:docker.NewProvider` — flow; flow compiler produced only entrypoint-driven flows
- `flows:CreateWithWaitForReady` — flow; flow compiler produced only entrypoint-driven flows
- `ownership:DefaultClusterName` — constant/interface/package; not a data store
- `ownership:providers.Provider` — constant/interface/package; not a data store
- `ownership:docker.NewProvider` — constant/interface/package; not a data store
- `ownership:podman.NewProvider` — constant/interface/package; not a data store
- `ownership:nerdctl.NewProvider` — constant/interface/package; not a data store
- `ownership:pkg/cluster/internal/kubeconfig` — constant/interface/package; not a data store
- `contracts:kind.x-k8s.io/v1alpha4` — config constant/contract; not rendered
- `contracts:--config` — CLI flag via cobra constructor functions; not extracted
- `contracts:--name` — CLI flag via cobra constructor functions; not extracted
- `contracts:--image` — CLI flag via cobra constructor functions; not extracted
- `contracts:--retain` — CLI flag via cobra constructor functions; not extracted
- `contracts:--wait` — CLI flag via cobra constructor functions; not extracted
- `contracts:--kubeconfig` — CLI flag via cobra constructor functions; not extracted
- `contracts:ControlPlaneRole NodeRole = "control-plane"` — config constant/contract; not rendered
- `contracts:WorkerRole NodeRole = "worker"` — config constant/contract; not rendered
- `tests:pkg/internal/apis/config/encoding/load_test.go` — test file path; no test inventory
- `tests:pkg/internal/apis/config/validate_test.go` — test file path; no test inventory
- `tests:pkg/internal/apis/config/cluster_util_test.go` — test file path; no test inventory
- `tests:pkg/cluster/internal/providers/docker/network_test.go` — test file path; no test inventory
- `tests:pkg/cluster/internal/providers/common/namer_test.go` — test file path; no test inventory
- `tests:pkg/cluster/internal/kubeconfig/internal/kubeconfig/write_test.go` — test file path; no test inventory
- `tests:pkg/cluster/internal/kubeconfig/internal/kubeconfig/remove_test.go` — test file path; no test inventory
- `tests:pkg/cluster/internal/kubeconfig/internal/kubeconfig/merge_test.go` — test file path; no test inventory

Format-fixed keys (0 rewritten to atlas notation in `benchmarks/ground-truth/`):


## microservices-demo

- `components:hipstershop` — proto package; no proto layer in atlas
- `entrypoints:src/checkoutservice/main.go` — entry file path; atlas renders symbols, not entry files
- `entrypoints:src/currencyservice/server.js` — entry file path; atlas renders symbols, not entry files
- `entrypoints:src/productcatalogservice/server.go` — entry file path; atlas renders symbols, not entry files
- `entrypoints:src/adservice/src/main/java/hipstershop/AdService.java` — entry file path; atlas renders symbols, not entry files
- `entrypoints:src/emailservice/email_server.py` — entry file path; atlas renders symbols, not entry files
- `entrypoints:src/recommendationservice/recommendation_server.py` — entry file path; atlas renders symbols, not entry files
- `entrypoints:src/shippingservice/main.go` — entry file path; atlas renders symbols, not entry files
- `entrypoints:src/frontend/main.go` — entry file path; atlas renders symbols, not entry files
- `entrypoints:src/cartservice/src/Program.cs` — entry file path; atlas renders symbols, not entry files
- `entrypoints:src/paymentservice/server.js` — entry file path; atlas renders symbols, not entry files
- `flows:ListProducts` -> catalog_loader.go -> products.json` — flow; flow compiler produced only entrypoint-driven flows
- `flows:getCart` -> CartService.GetCart` — flow; flow compiler produced only entrypoint-driven flows
- `flows:PlaceOrder` -> GetCart -> GetQuote -> Charge -> SendOrderConfirmation -> ShipOrder` — flow; flow compiler produced only entrypoint-driven flows
- `flows:AddItem` -> CartService -> cartstore` — flow; flow compiler produced only entrypoint-driven flows
- `flows:Convert` -> CurrencyService` — flow; flow compiler produced only entrypoint-driven flows
- `flows:GetAds` -> AdService` — flow; flow compiler produced only entrypoint-driven flows
- `flows:ListRecommendations` -> RecommendationService` — flow; flow compiler produced only entrypoint-driven flows
- `flows:homeHandler` -> getProducts -> ProductCatalogService.ListProducts` — flow; flow compiler produced only entrypoint-driven flows
- `ownership:RedisCartStore` — store/interface/file; not a data store
- `ownership:ICartStore` — store/interface/file; not a data store
- `ownership:SpannerCartStore` — store/interface/file; not a data store
- `ownership:src/productcatalogservice/products.json` — store/interface/file; not a data store
- `ownership:cookieSessionID` — store/interface/file; not a data store
- `ownership:skaffold.yaml` — store/interface/file; not a data store
- `contracts:rpc GetCart(GetCartRequest) returns (Cart)` — gRPC contract in demo.proto; no proto layer
- `contracts:rpc ListProducts(Empty) returns (ListProductsResponse)` — gRPC contract in demo.proto; no proto layer
- `contracts:rpc PlaceOrder(PlaceOrderRequest) returns (PlaceOrderResponse)` — gRPC contract in demo.proto; no proto layer
- `contracts:rpc GetSupportedCurrencies(Empty) returns (GetSupportedCurrenciesResponse)` — gRPC contract in demo.proto; no proto layer
- `contracts:containerPort: 9555` — port/container contract; not rendered
- `contracts:port: 7070` — port/container contract; not rendered
- `contracts:port: 5050` — port/container contract; not rendered
- `contracts:port: 50051` — port/container contract; not rendered
- `contracts:port: 3550` — port/container contract; not rendered
- `contracts:/_healthz` — route via r.HandleFunc; no route extraction for go
- `contracts:GET /product/{id}` — route via r.HandleFunc; no route extraction for go
- `tests:src/productcatalogservice/product_catalog_test.go` — test file path; no test inventory
- `tests:src/shippingservice/shippingservice_test.go` — test file path; no test inventory
- `tests:src/frontend/validator/validator_test.go` — test file path; no test inventory
- `tests:src/checkoutservice/money/money_test.go` — test file path; no test inventory
- `tests:src/frontend/money/money_test.go` — test file path; no test inventory
- `tests:src/loadgenerator/locustfile.py` — test file path; no test inventory

Format-fixed keys (0 rewritten to atlas notation in `benchmarks/ground-truth/`):


## mockito

- `components:MockitoCore` — class/interface symbol; java extractor disabled
- `components:ArgumentMatchers` — class/interface symbol; java extractor disabled
- `components:MockSettings` — class/interface symbol; java extractor disabled
- `components:Answers` — class/interface symbol; java extractor disabled
- `components:MockMaker` — class/interface symbol; java extractor disabled
- `components:ByteBuddyMockMaker` — class/interface symbol; java extractor disabled
- `components:InlineByteBuddyMockMaker` — class/interface symbol; java extractor disabled
- `components:MockUtil` — class/interface symbol; java extractor disabled
- `components:MockingProgress` — class/interface symbol; java extractor disabled
- `components:MockitoSession` — class/interface symbol; java extractor disabled
- `components:MockitoExtension` — class/interface symbol; java extractor disabled
- `entrypoints:mock(Class<T> classToMock)` — API method; java extractor disabled, no flow renders it
- `entrypoints:when(T methodCall)` — API method; java extractor disabled, no flow renders it
- `entrypoints:verify(T mock)` — API method; java extractor disabled, no flow renders it
- `entrypoints:MockitoAnnotations.openMocks` — API method; java extractor disabled, no flow renders it
- `entrypoints:mockitoSession()` — API method; java extractor disabled, no flow renders it
- `entrypoints:MockitoExtension.beforeEach` — API method; java extractor disabled, no flow renders it
- `entrypoints:MockitoJUnitRunner` — API method; java extractor disabled, no flow renders it
- `entrypoints:spy(T object)` — API method; java extractor disabled, no flow renders it
- `entrypoints:doReturn(Object toBeReturned)` — API method; java extractor disabled, no flow renders it
- `flows:mock()` -> MockitoCore.mock -> MockMaker.createMock` — flow; java disabled, no flows compiled
- `flows:when()` -> MockitoCore.when -> stubbingStarted() -> OngoingStubbing.thenReturn` — flow; java disabled, no flows compiled
- `flows:verify()` -> MockitoCore.verify -> VerificationModeFactory.times(1)` — flow; java disabled, no flows compiled
- `flows:any()`/`eq()` -> reportMatcher() -> matcher stack` — flow; java disabled, no flows compiled
- `flows:timeout(long millis)` — flow; java disabled, no flows compiled
- `flows:inOrder(mocks)` -> InOrder.verify` — flow; java disabled, no flows compiled
- `flows:MockitoExtension.beforeEach` — flow; java disabled, no flows compiled
- `flows:doThrow(Throwable...)` -> MOCKITO_CORE.stubber() -> Stubber.doThrow` — flow; java disabled, no flows compiled
- `ownership:MockingProgress` — state-holding class; not a data store (java disabled)
- `ownership:Plugins` — state-holding class; not a data store (java disabled)
- `ownership:MockitoSession` — state-holding class; not a data store (java disabled)
- `ownership:MockMakers` — state-holding class; not a data store (java disabled)
- `ownership:MockUtil` — state-holding class; not a data store (java disabled)
- `ownership:GlobalConfiguration` — state-holding class; not a data store (java disabled)
- `contracts:OngoingStubbing` — API contract; java disabled
- `contracts:VerificationMode` — API contract; java disabled
- `contracts:ArgumentMatcher<T>` — API contract; java disabled
- `contracts:ArgumentCaptor` — API contract; java disabled
- `contracts:@Mock` — annotation contract; no annotations layer
- `contracts:@InjectMocks` — annotation contract; no annotations layer
- `contracts:Strictness.STRICT_STUBS` — API contract; java disabled
- `contracts:MockSettings.extraInterfaces` — API contract; java disabled
- `contracts:@Captor` — annotation contract; no annotations layer
- `tests:MockitoTest` — test class/file path; no test inventory
- `tests:ArgumentCaptorTest` — test class/file path; no test inventory
- `tests:JunitJupiterTest` — test class/file path; no test inventory
- `tests:StrictnessTest` — test class/file path; no test inventory
- `tests:InjectMocksTest` — test class/file path; no test inventory
- `tests:verification` — test class/file path; no test inventory
- `tests:internal` — test class/file path; no test inventory
- `tests:StaticMockingExperimentTest` — test class/file path; no test inventory

Format-fixed keys (0 rewritten to atlas notation in `benchmarks/ground-truth/`):


## nest

- `components:NestFactoryStatic` — class/decorator symbol; no symbol inventory
- `components:NestFactory` — class/decorator symbol; no symbol inventory
- `components:Controller` — class/decorator symbol; no symbol inventory
- `components:Injectable` — class/decorator symbol; no symbol inventory
- `components:Param` / `Body` / `Query` — class/decorator symbol; no symbol inventory
- `components:ExpressAdapter` — class/decorator symbol; no symbol inventory
- `components:NestExpressApplication` — class/decorator symbol; no symbol inventory
- `components:ClientProxy` — class/decorator symbol; no symbol inventory
- `components:MessagePattern` — class/decorator symbol; no symbol inventory
- `components:TestingModuleBuilder` — class/decorator symbol; no symbol inventory
- `entrypoints:NestFactory.create(AppModule)` — bootstrap method; not in any rendered flow
- `entrypoints:NestFactory.createMicroservice` — bootstrap method; not in any rendered flow
- `entrypoints:NestFactory.createApplicationContext` — bootstrap method; not in any rendered flow
- `entrypoints:app.listen(port)` — bootstrap method; not in any rendered flow
- `entrypoints:Test.createTestingModule` — bootstrap method; not in any rendered flow
- `entrypoints:@Controller('cats')` — decorator; no annotations layer
- `entrypoints:packages/core/index.ts` — file path; atlas renders symbols, not entry files
- `flows:NestFactory.create` -> `NestApplication` — flow; flow compiler produced only entrypoint-driven flows
- `flows:DependenciesScanner.scan` — flow; flow compiler produced only entrypoint-driven flows
- `flows:Controller` -> `RoutesResolver` — flow; flow compiler produced only entrypoint-driven flows
- `flows:MessagePattern` -> server` — flow; flow compiler produced only entrypoint-driven flows
- `flows:Test.createTestingModule` -> `TestingModuleBuilder.compile` -> `TestingModule` — flow; flow compiler produced only entrypoint-driven flows
- `flows:NestContainer` -> `InstanceLoader` -> `Injector` — flow; flow compiler produced only entrypoint-driven flows
- `flows:ExpressAdapter` -> `RouterMethodFactory` — flow; flow compiler produced only entrypoint-driven flows
- `flows:NestFactory.create` -> `GraphInspector` — flow; flow compiler produced only entrypoint-driven flows
- `ownership:NestContainer` — DI container/state; not a data store
- `ownership:ModulesContainer` — DI container/state; not a data store
- `ownership:ApplicationConfig` — DI container/state; not a data store
- `ownership:InstanceLoader` — DI container/state; not a data store
- `ownership:Injector` — DI container/state; not a data store
- `ownership:GraphInspector` — DI container/state; not a data store
- `ownership:DependenciesScanner` — DI container/state; not a data store
- `contracts:@Controller('cats')` — decorator contract; no annotations layer
- `contracts:@Get(':id')` — decorator contract; no annotations layer
- `contracts:@Param('id', new ParseIntPipe())` — decorator contract; no annotations layer
- `contracts:@UseGuards(RolesGuard)` — decorator contract; no annotations layer
- `contracts:Test.createTestingModule` — API contract/enum; not rendered
- `contracts:NestExpressApplication` — API contract/enum; not rendered
- `contracts:@EventPattern` — decorator contract; no annotations layer
- `contracts:Transport` — API contract/enum; not rendered
- `contracts:APP_GUARD` — API contract/enum; not rendered
- `tests:packages/core/test` — test dir/file path; no test inventory
- `tests:integration/` — test dir/file path; no test inventory
- `tests:sample/01-cats-app/src/cats/cats.controller.spec.ts` — test dir/file path; no test inventory
- `tests:sample/19-auth-jwt/e2e/app/app.e2e-spec.ts` — test dir/file path; no test inventory
- `tests:sample/02-gateways/e2e/events-gateway/gateway.e2e-spec.ts` — test dir/file path; no test inventory
- `tests:sample/26-queues/e2e/audio/audio.e2e-spec.ts` — test dir/file path; no test inventory
- `tests:packages/common/test` — test dir/file path; no test inventory

Format-fixed keys (0 rewritten to atlas notation in `benchmarks/ground-truth/`):


## prettier

- `components:check` — function/doc-builder symbol; no symbol inventory
- `components:formatWithCursor` — function/doc-builder symbol; no symbol inventory
- `components:coreFormat` — function/doc-builder symbol; no symbol inventory
- `components:printAstToDoc` — function/doc-builder symbol; no symbol inventory
- `components:prepareToPrint` — function/doc-builder symbol; no symbol inventory
- `components:printDocToString` — function/doc-builder symbol; no symbol inventory
- `components:group` — function/doc-builder symbol; no symbol inventory
- `components:hardline` — function/doc-builder symbol; no symbol inventory
- `components:softline` — function/doc-builder symbol; no symbol inventory
- `components:builders` — function/doc-builder symbol; no symbol inventory
- `entrypoints:src/index.js` — file path entry; atlas renders symbols, not entry files
- `entrypoints:src/cli/index.js` — file path entry; atlas renders symbols, not entry files
- `entrypoints:src/standalone.js` — file path entry; atlas renders symbols, not entry files
- `entrypoints:formatFiles` — file path entry; atlas renders symbols, not entry files
- `entrypoints:src/cli/context.js` — file path entry; atlas renders symbols, not entry files
- `flows:format` -> `coreFormat` -> `parse` -> `printAstToDoc` -> `printDocToString` — flow; flow compiler produced only entrypoint-driven flows
- `flows:check` -> `format` text comparison` — flow; flow compiler produced only entrypoint-driven flows
- `flows:parse` -> `resolveParser` -> `parser.preprocess` — flow; flow compiler produced only entrypoint-driven flows
- `flows:printAstToDoc` -> `prepareToPrint` -> doc cache Map` — flow; flow compiler produced only entrypoint-driven flows
- `flows:formatFiles` -> `formatFile` -> `writeOutput` — flow; flow compiler produced only entrypoint-driven flows
- `flows:listDifferent` -> `prettier.check` — flow; flow compiler produced only entrypoint-driven flows
- `flows:formatFile` -> `mockable.writeFormattedFile` — flow; flow compiler produced only entrypoint-driven flows
- `flows:normalizeInputAndOptions` -> `normalizeFormatOptions` — flow; flow compiler produced only entrypoint-driven flows
- `ownership:printAstToDoc` — module state; not a data store
- `ownership:options` — module state; not a data store
- `ownership:resolveConfig` — module state; not a data store
- `ownership:context.argv` — module state; not a data store
- `ownership:mockable` — module state; not a data store
- `ownership:directory-ignorer` — module state; not a data store
- `contracts:--write` — CLI flag; prettier CLI options not extracted
- `contracts:--check` — CLI flag; prettier CLI options not extracted
- `contracts:--list-different` — CLI flag; prettier CLI options not extracted
- `contracts:--debug-check` — CLI flag; prettier CLI options not extracted
- `contracts:--log-level` — CLI flag; prettier CLI options not extracted
- `contracts:babel` — parser name/option constant; not rendered
- `contracts:CATEGORY_OUTPUT` — parser name/option constant; not rendered
- `tests:tests/format` — test dir/file path; no test inventory
- `tests:tests/unit` — test dir/file path; no test inventory
- `tests:tests/integration` — test dir/file path; no test inventory
- `tests:tests/config` — test dir/file path; no test inventory
- `tests:tests/dts` — test dir/file path; no test inventory
- `tests:tests/format/js` — test dir/file path; no test inventory
- `tests:tests/format/css` — test dir/file path; no test inventory
- `tests:tests/unit/doc-builders.js` — test dir/file path; no test inventory
- `tests:tests/unit/editorconfig-to-prettier.js` — test dir/file path; no test inventory

Format-fixed keys (0 rewritten to atlas notation in `benchmarks/ground-truth/`):


## pydantic

- `components:FieldInfo` — class/decorator symbol; no symbol inventory
- `components:ConfigDict` — class/decorator symbol; no symbol inventory
- `components:TypeAdapter` — class/decorator symbol; no symbol inventory
- `components:ValidationError` — class/decorator symbol; no symbol inventory
- `components:PydanticUserError` — class/decorator symbol; no symbol inventory
- `components:AfterValidator` — class/decorator symbol; no symbol inventory
- `components:BeforeValidator` — class/decorator symbol; no symbol inventory
- `components:field_validator` — class/decorator symbol; no symbol inventory
- `components:model_validator` — class/decorator symbol; no symbol inventory
- `components:computed_field` — class/decorator symbol; no symbol inventory
- `components:create_model` — class/decorator symbol; no symbol inventory
- `entrypoints:model_validate` — classmethod; atlas renders routes/entrypoints, not methods
- `entrypoints:model_validate_json` — classmethod; atlas renders routes/entrypoints, not methods
- `entrypoints:model_validate_strings` — classmethod; atlas renders routes/entrypoints, not methods
- `entrypoints:model_dump` — classmethod; atlas renders routes/entrypoints, not methods
- `entrypoints:model_dump_json` — classmethod; atlas renders routes/entrypoints, not methods
- `entrypoints:model_construct` — classmethod; atlas renders routes/entrypoints, not methods
- `entrypoints:model_json_schema` — classmethod; atlas renders routes/entrypoints, not methods
- `entrypoints:TypeAdapter.validate_python` — classmethod; atlas renders routes/entrypoints, not methods
- `entrypoints:TypeAdapter.dump_json` — classmethod; atlas renders routes/entrypoints, not methods
- `flows:__get_pydantic_core_schema__` — dunder/schema hook; no flow
- `flows:model_rebuild` — dunder/schema hook; no flow
- `flows:TypeAdapter.json_schema` — dunder/schema hook; no flow
- `flows:model_post_init` — dunder/schema hook; no flow
- `flows:__pydantic_init_subclass__` — dunder/schema hook; no flow
- `ownership:model_config` — class/instance attr; not a data store
- `ownership:__pydantic_fields__` — class/instance attr; not a data store
- `ownership:__pydantic_fields_set__` — class/instance attr; not a data store
- `ownership:model_extra` — class/instance attr; not a data store
- `ownership:__pydantic_private__` — class/instance attr; not a data store
- `ownership:ConfigDict.validate_assignment` — class/instance attr; not a data store
- `contracts:model_config = ConfigDict(validate_assignment=True)` — config/parameter contract; not rendered
- `contracts:strict=True` — config/parameter contract; not rendered
- `contracts:mode='json'` — config/parameter contract; not rendered
- `contracts:ConfigDict(defer_build=True)` — config/parameter contract; not rendered
- `contracts:indent` — config/parameter contract; not rendered
- `contracts:mode='validation'` — config/parameter contract; not rendered
- `tests:tests/test_main.py` — test file path; no test inventory
- `tests:tests/test_type_adapter.py` — test file path; no test inventory
- `tests:tests/test_json_schema.py` — test file path; no test inventory
- `tests:tests/test_validators.py` — test file path; no test inventory
- `tests:tests/test_model_validator.py` — test file path; no test inventory
- `tests:tests/test_dataclasses.py` — test file path; no test inventory
- `tests:tests/test_create_model.py` — test file path; no test inventory
- `tests:tests/test_errors.py` — test file path; no test inventory

Format-fixed keys (0 rewritten to atlas notation in `benchmarks/ground-truth/`):


## serde

- `components:Serializer` — trait symbol; no symbol inventory
- `components:Deserializer` — trait symbol; no symbol inventory
- `components:Visitor` — trait symbol; no symbol inventory
- `components:SerializeSeq` — trait symbol; no symbol inventory
- `components:SeqAccess` — trait symbol; no symbol inventory
- `components:IgnoredAny` — trait symbol; no symbol inventory
- `components:de::value` — trait symbol; no symbol inventory
- `entrypoints:#[derive(Serialize)]` — derive attribute; no derive-macro layer
- `entrypoints:#[derive(Deserialize)]` — derive attribute; no derive-macro layer
- `entrypoints:Serialize::serialize` — trait method / example usage; not in any rendered flow
- `entrypoints:Deserialize::deserialize` — trait method / example usage; not in any rendered flow
- `entrypoints:serde_json::to_string` — trait method / example usage; not in any rendered flow
- `entrypoints:serde_json::from_str` — trait method / example usage; not in any rendered flow
- `flows:Serialize.serialize` — flow; flow compiler produced only entrypoint-driven flows
- `flows:Deserializer.deserialize_struct` — flow; flow compiler produced only entrypoint-driven flows
- `flows:serde_json::to_string` — flow; flow compiler produced only entrypoint-driven flows
- `flows:Deserializer.deserialize_enum` — flow; flow compiler produced only entrypoint-driven flows
- `ownership:serde/src/private` — module path/state; not a data store
- `ownership:serde_core/src` — module path/state; not a data store
- `ownership:Content` — module path/state; not a data store
- `ownership:serde_core/src/format.rs` — module path/state; not a data store
- `contracts:serialize_struct` — API contract; not rendered
- `contracts:deserialize_struct` — API contract; not rendered
- `contracts:#[serde(tag = "t", content = "c")]` — derive attribute contract; no derive layer
- `contracts:Error::custom` — API contract; not rendered
- `contracts:serde_test::Token` — API contract; not rendered
- `contracts:serde_json::to_string` — API contract; not rendered
- `tests:test_suite/tests/test_ser.rs` — test file/dir path; no test inventory
- `tests:test_suite/tests/test_de.rs` — test file/dir path; no test inventory
- `tests:test_suite/tests/test_roundtrip.rs` — test file/dir path; no test inventory
- `tests:test_suite/tests/test_annotations.rs` — test file/dir path; no test inventory
- `tests:test_suite/tests/test_enum_adjacently_tagged.rs` — test file/dir path; no test inventory
- `tests:test_suite/tests/regression/issue2565.rs` — test file/dir path; no test inventory
- `tests:test_suite/tests/ui` — test file/dir path; no test inventory

Format-fixed keys (0 rewritten to atlas notation in `benchmarks/ground-truth/`):


## shadcn-ui

- `components:buttonVariants` — TS component symbol; no symbol inventory
- `components:Card` — TS component symbol; no symbol inventory
- `components:Dialog` — TS component symbol; no symbol inventory
- `components:apply` — TS component symbol; no symbol inventory
- `components:@shadcn/react` — package name; no package layer
- `components:@shadcn/helpers` — package name; no package layer
- `components:apps/v4/source.config.ts` — config file path; no atlas form
- `entrypoints:npx shadcn init` — CLI invocation string; no atlas form
- `entrypoints:shadcn add` — CLI invocation string; no atlas form
- `entrypoints:npx shadcn create` — CLI invocation string; no atlas form
- `entrypoints:next dev --turbopack --port 4000` — CLI invocation string; no atlas form
- `entrypoints:scripts/build-registry.mts` — entry file path; atlas renders symbols, not entry files
- `entrypoints:apps/v4/app/(app)/docs/[[...slug]]/page.tsx` — entry file path; atlas renders symbols, not entry files
- `entrypoints:apps/v4/app/(app)/blocks/[...categories]/page.tsx` — entry file path; atlas renders symbols, not entry files
- `entrypoints:apps/v4/app/api/search/route.ts` — entry file path; atlas renders symbols, not entry files
- `flows:build-registry.mts` — flow; flow compiler produced only entrypoint-driven flows
- `flows:add` command -> fetch item address -> apply component to project` — flow; flow compiler produced only entrypoint-driven flows
- `flows:components.json` — flow; flow compiler produced only entrypoint-driven flows
- `flows:packages/shadcn/src/commands/apply.ts` — flow; flow compiler produced only entrypoint-driven flows
- `flows:apps/v4/lib/source.ts` — flow; flow compiler produced only entrypoint-driven flows
- `flows:createFromSource` — flow; flow compiler produced only entrypoint-driven flows
- `flows:capture-registry.mts` — flow; flow compiler produced only entrypoint-driven flows
- `flows:capture-explore.mts` — flow; flow compiler produced only entrypoint-driven flows
- `ownership:apps/v4/registry.json` — file/registry path; not a data store
- `ownership:apps/v4/components.json` — file/registry path; not a data store
- `ownership:apps/v4/lib/registry.ts` — file/registry path; not a data store
- `ownership:apps/v4/lib/config.ts` — file/registry path; not a data store
- `ownership:registry/new-york-v4` — file/registry path; not a data store
- `ownership:registry/bases` — file/registry path; not a data store
- `ownership:apps/v4/lib/themes.ts` — file/registry path; not a data store
- `contracts:"button"` — registry item/slot contract; not rendered
- `contracts:apps/v4/registry/new-york-v4/ui/card.tsx` — registry item/slot contract; not rendered
- `contracts:apps/v4/registry/new-york-v4/ui/dialog.tsx` — registry item/slot contract; not rendered
- `contracts:npx shadcn init` — registry item/slot contract; not rendered
- `contracts:shadcn add` — registry item/slot contract; not rendered
- `contracts:--yes` — registry item/slot contract; not rendered
- `contracts:data-slot="dialog"` — registry item/slot contract; not rendered
- `contracts:data-slot="card"` — registry item/slot contract; not rendered
- `contracts:[[...slug]]` — registry item/slot contract; not rendered
- `tests:packages/shadcn/src/commands/add.test.ts` — test file/dir path; no test inventory
- `tests:packages/shadcn/src/commands/init.test.ts` — test file/dir path; no test inventory
- `tests:packages/shadcn/src/commands/build.test.ts` — test file/dir path; no test inventory
- `tests:packages/shadcn/src/commands/apply.test.ts` — test file/dir path; no test inventory
- `tests:packages/shadcn/test` — test file/dir path; no test inventory
- `tests:apps/v4/registry/calendar.test.ts` — test file/dir path; no test inventory
- `tests:apps/v4/registry/config.test.ts` — test file/dir path; no test inventory
- `tests:packages/tests` — test file/dir path; no test inventory

Format-fixed keys (0 rewritten to atlas notation in `benchmarks/ground-truth/`):


## sqlalchemy

- `components:Connection` — class symbol; no symbol inventory
- `components:sessionmaker` — class symbol; no symbol inventory
- `components:scoped_session` — class symbol; no symbol inventory
- `components:Mapper` — class symbol; no symbol inventory
- `components:ForeignKey` — class symbol; no symbol inventory
- `components:SQLiteDialect` — class symbol; no symbol inventory
- `components:PGDialect` — class symbol; no symbol inventory
- `components:MySQLDialect` — class symbol; no symbol inventory
- `entrypoints:create_engine` — function/method; not in any rendered flow
- `entrypoints:declarative_base` — function/method; not in any rendered flow
- `entrypoints:Engine.connect` — function/method; not in any rendered flow
- `entrypoints:Connection.execute` — function/method; not in any rendered flow
- `entrypoints:Session.execute` — function/method; not in any rendered flow
- `entrypoints:Session.connection` — function/method; not in any rendered flow
- `entrypoints:MetaData.create_all` — function/method; not in any rendered flow
- `flows:create_engine -> Engine.connect -> Connection.execute` — flow; flow compiler produced only entrypoint-driven flows
- `flows:Session.execute -> ORMExecuteState` — flow; flow compiler produced only entrypoint-driven flows
- `flows:declarative_base -> Mapper.configure` — flow; flow compiler produced only entrypoint-driven flows
- `flows:Transaction -> commit/rollback` — flow; flow compiler produced only entrypoint-driven flows
- `flows:QueuePool -> _ConnectionFairy` — flow; flow compiler produced only entrypoint-driven flows
- `ownership:engine.pool` — config/identity-map state; not a data store
- `ownership:Session identity map` — config/identity-map state; not a data store
- `ownership:MetaData.tables` — config/identity-map state; not a data store
- `ownership:sessionmaker(bind=engine)` — config/identity-map state; not a data store
- `ownership:_sa_instance_state` — config/identity-map state; not a data store
- `ownership:Session.connection` — config/identity-map state; not a data store
- `contracts:create_engine("sqlite://")` — API contract; not rendered
- `contracts:"sqlite:///:memory:"` — API contract; not rendered
- `contracts:LABEL_STYLE_TABLENAME_PLUS_COL` — API contract; not rendered
- `contracts:select(t1).where(t1.c.c2 == t2.c.c1)` — API contract; not rendered
- `contracts:test_session_commit_rollback` — API contract; not rendered
- `contracts:name = "sqlite"` — API contract; not rendered
- `tests:test/orm/test_session.py` — test file/dir path; no test inventory
- `tests:test/orm/test_query.py` — test file/dir path; no test inventory
- `tests:test/engine/test_transaction.py` — test file/dir path; no test inventory
- `tests:test/engine/test_pool.py` — test file/dir path; no test inventory
- `tests:test/dialect/sqlite/` — test file/dir path; no test inventory
- `tests:test/aaa_profiling/test_orm.py` — test file/dir path; no test inventory
- `tests:test/aaa_profiling/test_pool.py` — test file/dir path; no test inventory

Format-fixed keys (2 rewritten to atlas notation in `benchmarks/ground-truth/`):
- `flows:Table -> MetaData.create_all` -> `Table`
- `contracts:Table("sometable", m, Column("somecolumn", String))` -> `Table`
