# gorilla-mux
> https://github.com/gorilla/mux | Go | go service (lib) | ~8k LOC

## architecture
- `Router` — main router struct in mux.go:54; holds routes, middleware, NotFoundHandler, and route config
- `Route` — route struct in route.go:17; a single matcher+handler registration, also builds URLs

## entrypoints
- `NewRouter` — mux.go:32; creates a Router with a namedRoutes map
- `Router.HandleFunc` — mux.go:335; `HandlerFunc` shorthand (Path(tpl).HandlerFunc(f)), the classic registration entry
- `Router.Handle` — mux.go:329; handler-typed registration
- `Router.NewRoute` — mux.go:314; appends an empty route to build matchers on
- `Router.ServeHTTP` — mux.go:188; http.Handler entry; matches request then runs handler/middleware chain
- `Router.Match` — mux.go:151; iterates routes until one matches, fills RouteMatch

## behavior
- `NewRouter` — registration to dispatch (mux_test.go:2143): NewRouter -> r.HandleFunc("/api", h).Methods("POST") -> router.ServeHTTP
- `ServeHTTP` — matching pipeline (mux.go:151-188): ServeHTTP -> Match -> Route.Match -> routeRegexpGroup matchers -> handler
- `PathPrefix` — subrouter creation inheriting parent routeConf (route.go:557): PathPrefix("/sub/") -> Subrouter -> s.HandleFunc
- `Vars` — extract path variables from request context (mux.go:466): Vars(r) -> r.Context().Value(varsKey)
- `Router.Use` — middleware wrapping (middleware.go:24): Router.Use -> ServeHTTP -> middleware chain -> final handler

## state_authority
- `Router.routes` — owns the appended Route list (mux.go:317)
- `Router.namedRoutes` — name-to-Route map for URL building; `Route.Name` registers into it (route.go:212)
- `varsKey` — context key (mux.go:460-462; with `routeKey`/`routerKey`) carrying per-request match state
- `routeRegexp` — owns compiled template, varsN list, and regexp per matcher
- `Router.routeConf` — configuration copied into every new route/subrouter (mux.go:82)

## contracts
- `r.HandleFunc("/api", emptyHandler).Methods("POST")` — method-restricted route in mux_test.go:2143
- `Queries("time", "{time:[0-9]+}")` — query-param regex contract (mux_test.go:2144)
- `Queries("foo", "{foo:[0-9]+}")` — named query var matched via `{name:pattern}` syntax (mux_test.go:2190)
- `PathPrefix("/sub/").Subrouter()` — prefix subrouting contract (mux_test.go:2209)
- `Subrouter` — method-first subrouter pattern (mux_test.go:2286): Methods("GET").Subrouter().HandleFunc("/foo", ...)
- `Host("{subdomain}.domain.com")` — host template contract in route_test.go:67
- `r.HandleFunc("/", func1).Name("func1")` — named routes for URL building (mux_test.go:2036)
- `{name:pattern}` — brace template syntax parsed by `newRouteRegexp` (regexp.go:41)

## landmarks
- `RouteMatch` — match result struct in mux.go:446 (Route, Handler, Vars, MatchErr)
- `routeRegexp` — compiled template matcher in regexp.go:169
- `routeRegexpGroup` — grouping of host/path/queries matchers in regexp.go:332
- `routeConf` — shared configuration (useEncodedPath, strictSlash, skipClean) copied to subrouters in mux.go:82
- `MatcherFunc` — custom matcher signature in route.go:374
- `MiddlewareFunc` — `func(http.Handler) http.Handler` middleware signature in middleware.go:11

## tests
- `mux_test.go` — route matching, methods, queries, subrouters, and context vars
- `route_test.go` — Route building and URL generation (`Queries`, `Host`)
- `regexp_test.go` — template/brace parsing and regexp compilation tests
- `example_route_test.go` — runnable examples (path/query/host matching)
- `example_authentication_middleware_test.go` — middleware-chain example
- `example_cors_method_middleware_test.go` — CORS via methods middleware example
- `mux_httpserver_test.go` — ServeHTTP against httptest servers
- `bench_test.go` — route-matching benchmarks
