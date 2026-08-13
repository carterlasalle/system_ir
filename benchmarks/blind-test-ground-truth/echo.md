# echo
> https://github.com/labstack/echo | Go | go web framework | ~25k LOC

## architecture
- echo.go — the core: Echo, New, router setup (echo.go)
- context.go — the context: Context, JSON/HTML helpers (context.go)
- group.go — route groups: Group (group.go)
- binder.go — request binding: binder, Bind (binder.go)
- router.go — the router: Router, Route, add (router.go)
- middleware — the middleware collection: logger, recover, cors, jwt, static, compress, gzip, rate-limiter (middleware/)
- json.go — JSON handling: DefaultJSONSerializer (json.go)
- renderer.go — the renderer interface (renderer.go)
- httperror.go — the HTTPError type (httperror.go)
- ip.go — IP utilities (ip.go)
- bind.go — binding helpers (bind.go)
- response.go — the response writer (response.go)

## entrypoints
- echo.New — the echo factory (echo.go)
- e.GET — GET route registration
- e.POST — POST route registration
- e.PUT — PUT route registration
- e.DELETE — DELETE route registration
- e.PATCH — PATCH route registration
- e.HEAD — HEAD route registration
- e.OPTIONS — OPTIONS route registration
- e.CONNECT — CONNECT route registration
- e.TRACE — TRACE route registration
- e.Any — all-methods route registration
- e.Pre — pre-middleware registration
- e.Use — middleware registration
- e.Group — route group creation
- e.Static — static file serving
- e.Start — start the server
- e.StartTLS — start with TLS
- e.Router — the router accessor
- e.Routes — the route list
- e.Logger — the logger accessor
- e.ServeHTTP — the http handler

## behavior
- e.Start -> http.Server -> ServeHTTP -> router (echo.go)
- e.GET(path, handler) -> router.Add -> match (router.go)
- e.Use(middleware) -> middleware chain wrap (echo.go)
- e.Group("/api") -> prefixed sub-router (group.go)
- Context.Bind -> binder -> struct (binder.go)
- c.JSON(code, data) -> serialize -> response (context.go)
- e.Pre(middleware) -> early middleware (echo.go)
- router.match -> route -> handler chain (router.go)
- e.Static("/assets", dir) -> file server (echo.go)

## state_authority
- Echo — the app state: routes, middleware, config (echo.go)
- Context — the per-request state: params, query, response (context.go)
- Router — the routing state (router.go)
- Route — the route state (router.go)
- Group — the group state (group.go)
- HTTPError — the error state (httperror.go)
- Binder — the binder state (binder.go)
- response — the response writer state (response.go)
- Config — the app configuration (echo.go)

## contracts
- e.GET("/", handler) — GET route contract
- e.POST("/users", handler) — POST route contract
- e.PUT("/users/:id", handler) — PUT route contract
- e.DELETE("/users/:id", handler) — DELETE route contract
- e.PATCH("/users/:id", handler) — PATCH route contract
- e.Any("/x", handler) — any-method contract
- e.Use(middleware) — middleware contract
- e.Group("/api") — group contract
- e.Static("/static", "dir") — static contract
- e.Start(":8080") — start contract
- :id — path parameter contract
- c.JSON(200, obj) — json contract
- c.String(200, "text") — string contract
- c.Bind(&body) — bind contract
- c.Param("id") — param access contract
- c.QueryParam("q") — query access contract
- c.Request() — request access contract
- c.Response() — response access contract

## landmarks
- Echo — the app struct (echo.go)
- New — the app factory (echo.go)
- Context — the context struct (context.go)
- Router — the router (router.go)
- Route — the route struct (router.go)
- Group — the group (group.go)
- HTTPError — the error type (httperror.go)
- Binder — the binder (binder.go)
- DefaultBinder — the default binder (binder.go)
- HandlerFunc — the handler type (echo.go)
- MiddlewareFunc — the middleware type (echo.go)
- DefaultJSONSerializer — the json serializer (json.go)

## tests
- echo_test.go — core tests
- context_test.go — context tests
- router_test.go — router tests
- group_test.go — group tests
- binder_test.go — binder tests
- bind_test.go — bind tests
- json_test.go — json tests
- middleware/middleware_test.go — middleware tests
- middleware/logger/logger_test.go — logger middleware tests
- middleware/jwt/jwt_test.go — jwt middleware tests
