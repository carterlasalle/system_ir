# gin
> https://github.com/gin-gonic/gin | Go | go service | ~24k LOC

## architecture
- `Engine` — core server struct in gin.go:92; holds router trees, middleware, and HTTP config
- `RouterGroup` — route-grouping struct in routergroup.go:55; base of the route-registration API
- `Context` — per-request struct in context.go:61; request/response handle passed to handlers
- `binding` — package (binding.JSON etc.) backing `ShouldBindJSON` and `Bind`
- `render` — package (render.JSON, render.String, ...) backing `Context.JSON`/`String`/`HTML`
- `Logger` — logging middleware in logger.go:224
- `Recovery` — panic-recovery middleware in recovery.go:35

## entrypoints
- `gin.New()` — gin.go:202; bare Engine with no middleware
- `gin.Default()` — gin.go:236; Engine with Logger + Recovery middleware preinstalled
- `Engine.Run` — gin.go:540; attaches to an addr and serves HTTP (blocking)
- `Engine.ServeHTTP` — gin.go:662; http.Handler entry used by net/http
- `RouterGroup.GET` — routergroup.go:116; registers a GET route
- `RouterGroup.POST` — routergroup.go:111; registers a POST route
- `RouterGroup.Group` — routergroup.go:72; creates a path-prefixed sub-group
- `RouterGroup.Use` — routergroup.go:65; appends middleware to the group chain

## behavior
- `gin.Default` — canonical app flow: gin.Default -> r.GET("/ping", handler) -> r.Run (create engine, register routes, serve)
- `ServeHTTP` — request dispatch through the radix tree (gin.go:662-690): ServeHTTP -> handleHTTPRequest -> methodTree lookup -> HandlersChain -> Context.Next
- `Use` — middleware registration updating fallback handlers (gin.go:340): Use(middleware) -> rebuild404Handlers -> rebuild405Handlers
- `Context.JSON` — JSON response path (context.go:1255): Context.JSON(code, obj) -> render.JSON -> Render
- `RouterGroup.handle` — route registration funnel for all HTTP methods (routergroup.go:86): RouterGroup.handle -> engine.addRoute
- `Context.Next` — middleware chain advancement (context.go:198): Context.Next -> handler chain iteration -> c.index

## state_authority
- `engine.trees` — owns per-method radix trees; `Routes()` iterates them (gin.go:390)
- `engine.RouterGroup` — embedded RouterGroup owns registered routes and middleware
- `engine.noRoute` — 404 handler ownership (`engine.allNoRoute` variant), rebuilt on middleware change
- `Context` — owns request-scoped values (keys/values via Set/Get) and the handler-chain index
- `RouterGroup.BasePath` — group path-prefix state (routergroup.go:82)

## contracts
- `router.GET("/ping"` — canonical route example: `router.GET("/ping", ...)` in benchmarks_test.go, doc.go, README.md
- `GET /test` — test route with `c.HTML`/`c.String` handlers in gin_test.go:44
- `GET /:count` — named path-param route read via `c.Param("count")` in gin_test.go:680
- `GET /v1/:path` — escaped-path param route tests in gin_test.go:823
- `GET /test/:param` — param route with literal-colon sibling `GET /test\:action` in gin_test.go:1123
- `POST /` — method-restricted registration; `Any`/`Match`/`HEAD`/`OPTIONS` variants on RouterGroup (routergroup.go:111-156)
- `Engine.NoRoute` — custom 404 handlers contract (gin.go:326)
- `Context.Param` — path-var accessor (context.go:513); `Context.Query` — query-string accessor (context.go:535)
- `Context.ShouldBindJSON` — JSON body binding contract (context.go:890)

## landmarks
- `HandlerFunc` — handler signature `func(*Context)` in gin.go:51
- `HandlersChain` — slice of HandlerFunc in gin.go:57; middleware + final handler per route
- `RouteInfo` — gin.go:68 (slice type `RoutesInfo` at :76); route metadata (Method, Path, Handler) returned by `Engine.Routes`
- `methodTree` — radix-tree node wrapper per HTTP method in tree.go:45; the route lookup structure
- `H` — `map[string]any` shortcut for JSON responses in utils.go:61
- `Error` — error recording types in errors.go for the error chain (`ErrorType` is the category enum)

## tests
- `gin_test.go` — route registration, params, middleware, and renderer tests
- `gin_integration_test.go` — end-to-end server tests (HTML templates, redirects, HTTP/2)
- `benchmarks_test.go` — `GET /ping` route benchmarks (BenchmarkOneRoute, Benchmark404, ...)
- `context_test.go` — Context getters/setters and request-scoped value tests
- `auth_test.go` — BasicAuth middleware tests
- `errors_test.go` — Error chain and ErrorType tests
- `router.Group("/users")` — group coverage: `v1.GET("/test")` sub-group tests (gin_test.go:616, 718) and `route.GET("/test", ...)` BindQuery test (gin_test.go:1039)
