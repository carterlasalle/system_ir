# hono
> https://github.com/honojs/hono | TypeScript | ts web framework | ~30k LOC

## architecture
- src — the framework root: Hono, HonoRequest, Context, router (src/)
- hono-base.ts — the core: the Hono class (src/hono-base.ts)
- hono.ts — the framework entry with default router (src/hono.ts)
- context.ts — the Context class: request/response helpers (src/context.ts)
- request.ts — the HonoRequest class (src/request.ts)
- compose.ts — middleware composition: compose (src/compose.ts)
- router — the router backends: RegExpRouter, TrieRouter, PatternRouter, LinearRouter, SmartRouter (src/router/)
- middleware — the middleware collection: cors, jwt, basic-auth, bearer-auth, logger, etag, serve-static, timeout, request-id, secure-headers (src/middleware/)
- helper — the helper collection: html, cookie, css, ssg, streaming, websocket, proxy, factory (src/helper/)
- validator — request validation: validator, body, query, header, cookie, json (src/validator/)
- adapter — platform adapters (src/adapter/)
- client — the RPC client (src/client/)
- jsx — the JSX runtime (src/jsx/)
- preset — presets (src/preset/)

## entrypoints
- new Hono() — the app entry
- app.get — GET route registration
- app.post — POST route registration
- app.put — PUT route registration
- app.delete — DELETE route registration
- app.patch — PATCH route registration
- app.options — OPTIONS route registration
- app.all — all-methods route registration
- app.use — middleware registration
- app.route — route group/sub-app mounting
- app.basePath — base path scoping
- app.onError — error handler
- app.notFound — not-found handler
- app.fire — start the server
- app.mount — mount another handler
- app.fetch — the fetch handler entry
- HonoRequest — the request wrapper entry
- Context — the context entry
- compose — middleware composition entry

## behavior
- app.fetch -> router match -> handler chain -> Response — request dispatch (hono-base.ts)
- app.get(path, ...handlers) -> route registration -> router (hono-base.ts)
- compose(middlewares) -> dispatch chain — middleware composition (compose.ts)
- app.route("/api", subApp) -> route merge (hono-base.ts)
- validator -> parse -> context — request validation (validator/)
- Context.json -> Response — response building (context.ts)
- app.onError -> error -> Response — error flow (hono-base.ts)
- notFound -> 404 Response — miss flow (hono-base.ts)

## state_authority
- Hono — the app state: routes, middleware, router (hono-base.ts)
- Context — per-request state: env, execution context, response (context.ts)
- HonoRequest — the request state (request.ts)
- Router — the routing state (router/)
- RegExpRouter — the regex router state (router/reg-exp-router/)
- TrieRouter — the trie router state (router/trie-router/)
- SmartRouter — the router chooser state (router/smart-router/)
- ContextStorage — per-context storage (middleware/context-storage/)

## contracts
- app.get("/", c => c.text("hi")) — GET route contract
- app.post("/users", handler) — POST route contract
- app.put("/users/:id", handler) — PUT route contract
- app.delete("/users/:id", handler) — DELETE route contract
- app.use("*", middleware) — middleware contract
- app.route("/api", subApp) — route group contract
- app.basePath("/v1") — base path contract
- app.onError(handler) — error contract
- app.notFound(handler) — 404 contract
- :id — path parameter contract
- c.text("hi") — text response contract
- c.json({ok: true}) — json response contract
- c.html("<p>hi</p>") — html response contract
- c.req.param("id") — param access contract
- c.req.query("q") — query access contract
- c.req.header("x") — header access contract
- new Hono().fetch(req) — fetch contract

## landmarks
- Hono — the app class (hono-base.ts)
- HonoRequest — the request class (request.ts)
- Context — the context class (context.ts)
- compose — the composer (compose.ts)
- RegExpRouter — the regex router (router/reg-exp-router/)
- TrieRouter — the trie router (router/trie-router/)
- SmartRouter — the smart router (router/smart-router/)
- cors — the CORS middleware (middleware/cors/)
- jwt — the JWT middleware (middleware/jwt/)
- serveStatic — the static middleware (middleware/serve-static/)
- validator — the validator (validator/validator.ts)
- streaming — the streaming helper (helper/streaming/)

## tests
- src/hono.test.ts — app tests
- src/compose.test.ts — compose tests
- src/context.test.ts — context tests
- src/request.test.ts — request tests
- src/router/*/*.test.ts — router backend tests
- src/middleware/*/*.test.ts — middleware tests
- src/helper/*/*.test.ts — helper tests
- src/validator/*.test.ts — validator tests
