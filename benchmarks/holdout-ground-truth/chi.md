# chi
> https://github.com/go-chi/chi | Go | go service (router lib) | ~13k LOC

## architecture
- chi.go — the router: Mux, Router interface, NewRouter (chi.go)
- mux.go — the Mux implementation: routing tree, middleware chain (mux.go)
- tree.go — the radix tree route matcher (tree.go)
- context.go — per-request routing context: URLParams (context.go)
- chain.go — middleware chain helpers (chain.go)
- middleware — the bundled middleware package: Logger, Recoverer, RequestID (middleware/)
- _examples — example applications (_examples/)

## entrypoints
- NewRouter — router construction entry (chi.go)
- r.Get — GET route registration
- r.Post — POST route registration
- r.Put — PUT route registration
- r.Delete — DELETE route registration
- r.Patch — PATCH route registration
- r.Use — middleware registration
- r.With — inline-scoped middleware chain
- r.Route — route group sub-router
- r.Mount — sub-router mounting
- r.NotFound — 404 handler registration
- http.ListenAndServe — server start (via net/http)

## behavior
- Mux.ServeHTTP -> FindRoute -> tree lookup -> handler dispatch — request dispatch (mux.go)
- FindRoute -> radix tree match -> URLParams capture — route matching (tree.go)
- Mux.handle -> buildChain -> middlewares wrap handler — middleware chaining (mux.go)
- r.Route(pattern, fn) -> NewRouter -> Mount — sub-router creation
- context.WithURLParams -> set URL params in context — param storage (context.go)
- Chain.Handler -> wrap each middleware — chain assembly (chain.go)
- NotFound handler -> default 404 — miss handling

## state_authority
- Mux — the router state: routes tree, middleware stack (mux.go)
- node — the radix tree node (tree.go)
- URLParams — per-request captured params (context.go)
- RouteContext — the routing context on the request (context.go)
- m.middlewares — middleware chain state (mux.go)
- NotFoundHandler — custom 404 state

## contracts
- r.Get("/user/{name}", handler) — GET route contract
- r.Route("/articles", func(r chi.Router) {...}) — sub-route group contract
- r.Use(middleware.Logger) — middleware contract
- r.With(middleware.Timeout(60*time.Second)).Get(...) — scoped middleware contract
- r.Mount("/admin", subRouter) — mount contract
- /user/{name} — named path parameter contract
- /user/{name:[a-z]+} — regex parameter contract
- /page/* — wildcard catch-all contract
- chi.URLParam(r, "name") — param access contract (context.go)
- r.Method("GET", "/path", handler) — explicit method contract
- r.NotFound(handler) — custom 404 contract

## landmarks
- NewMux — mux factory (mux.go)
- Router — the router interface (chi.go)
- Mux — the mux implementation (mux.go)
- RouteContext — request routing context (context.go)
- URLParams — param store (context.go)
- Chain — middleware chain builder (chain.go)
- methodNotAllowedHandler — 405 handling
- middleware.RequestID — request ID middleware

## tests
- mux_test.go — mux routing tests
- tree_test.go — radix tree tests
- context_test.go — context/param tests
- chain_test.go — middleware chain tests
- middleware/ — middleware package tests
