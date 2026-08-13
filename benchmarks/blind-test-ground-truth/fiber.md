# fiber
> https://github.com/gofiber/fiber | Go | go web framework | ~50k LOC

## architecture
- app.go — the core: App, New, Config, error handling (app.go)
- router.go — the routing: route registration, tree (router.go)
- ctx.go — the context: Ctx, request/response access (ctx.go)
- group.go — route groups: Group (group.go)
- mount.go — sub-app mounting (mount.go)
- listen.go — the server: Listen, TLS (listen.go)
- middleware — the bundled middleware: logger, recover, cors, helmet (middleware/)
- client — the HTTP client: Client (client/)
- bind.go — request binding: Bind (bind.go)
- adapter.go — net/http adapter (adapter.go)
- prefork.go — prefork mode (prefork.go)
- storage_interface.go — storage abstraction (storage_interface.go)
- internal — internal utilities (internal/)
- docs — documentation (docs/)

## entrypoints
- fiber.New — the app factory (app.go)
- app.Get — GET route registration
- app.Post — POST route registration
- app.Put — PUT route registration
- app.Delete — DELETE route registration
- app.Patch — PATCH route registration
- app.Head — HEAD route registration
- app.Options — OPTIONS route registration
- app.All — all-methods route registration
- app.Add — method-list route registration
- app.Use — middleware registration
- app.Group — route group creation
- app.Route — route group with prefix
- app.Mount — sub-app mounting
- app.Domain — host-scoped routing
- app.Listen — start the HTTP server
- app.ListenTLS — start with TLS
- app.GetRoutes — route inventory
- app.Config — the app configuration
- app.Handler — the fasthttp handler
- fiber.NewError — error creation

## behavior
- app.Listen -> fasthttp server -> handler dispatch (listen.go)
- app.Get(path, handler) -> route tree insert -> request match (router.go)
- app.Use(middleware) -> middleware chain -> handler (app.go)
- app.Group("/api") -> prefixed routes (group.go)
- app.Mount("/v2", subApp) -> sub-app delegation (mount.go)
- c.JSON(data) -> serialize -> response (ctx.go)
- c.BodyParser -> bind -> validation (bind.go)
- app.GetRoutes -> route inventory (app.go)
- request -> app.handler -> Ctx pool -> response (app.go)

## state_authority
- App — the app state: routes, config, middleware (app.go)
- Config — the app configuration state (app.go)
- Ctx — the per-request state: params, query, body, locals (ctx.go)
- Router — the routing state (router.go)
- Group — the group state (group.go)
- Route — the route state (router.go)
- Error — the error state (error.go)
- Storage — the storage backend state (storage_interface.go)
- TLSHandler — the TLS state (app.go)

## contracts
- app.Get("/", handler) — GET route contract
- app.Post("/users", handler) — POST route contract
- app.Put("/users/:id", handler) — PUT route contract
- app.Delete("/users/:id", handler) — DELETE route contract
- app.Patch("/users/:id", handler) — PATCH route contract
- app.Use(middleware) — middleware contract
- app.Group("/api") — group contract
- app.Mount("/v2", subApp) — mount contract
- app.Listen(":3000") — listen contract
- :id — path parameter contract
- c.JSON(obj) — json response contract
- c.BodyParser(&body) — body parse contract
- c.Params("id") — param access contract
- c.Query("q") — query access contract
- c.SendString("text") — text response contract
- c.Status(404) — status contract
- app.Config() — config contract

## landmarks
- App — the app struct (app.go)
- New — the app factory (app.go)
- Ctx — the context struct (ctx.go)
- Router — the router (router.go)
- Group — the group (group.go)
- Config — the config struct (app.go)
- Handler — the handler type (app.go)
- Error — the error type (error.go)
- ErrorHandler — the error handler type (app.go)
- Client — the http client (client/)
- DefaultErrorHandler — the default error handler (app.go)
- Views — the view renderer interface (app.go)

## tests
- app_test.go — app tests
- router_test.go — router tests
- ctx_test.go — context tests
- group_test.go — group tests
- listen_test.go — listen tests
- bind_test.go — bind tests
- middleware/logger/logger_test.go — logger middleware tests
- middleware/recover/recover_test.go — recover middleware tests
- client/client_test.go — client tests
