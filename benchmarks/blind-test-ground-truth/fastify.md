# fastify
> https://github.com/fastify/fastify | TypeScript/JavaScript | ts backend framework | ~100k LOC

## architecture
- fastify.js — the framework core: fastify(), the instance factory (fastify.js)
- lib/route.js — routing: buildRouting, findRoute, route, routeHandler (lib/route.js)
- lib/server.js — the Node server wrapper: createServer (lib/server.js)
- lib/reply.js — the reply object: Reply (lib/reply.js)
- lib/request.js — the request object: Request (lib/request.js)
- lib/hooks.js — lifecycle hooks: onRequest, preHandler, onSend, onError (lib/hooks.js)
- lib/context.js — per-route context (lib/context.js)
- lib/schemas.js — JSON schema store (lib/schemas.js)
- lib/validation.js — payload/querystring/params validation (lib/validation.js)
- lib/content-type-parser.js — body parsing (lib/content-type-parser.js)
- lib/error-handler.js — error handling (lib/error-handler.js)
- lib/four-oh-four.js — 404 handling (lib/four-oh-four.js)
- lib/decorate.js — decoration: decorator API (lib/decorate.js)
- lib/plugin-utils.js — plugin metadata (lib/plugin-utils.js)
- lib/logger-pino.js — the pino logger (lib/logger-pino.js)
- types — TypeScript definitions (types/)
- test — the test suite (test/)

## entrypoints
- fastify() — the server factory (fastify.js)
- fastify.get — GET route registration
- fastify.post — POST route registration
- fastify.put — PUT route registration
- fastify.delete — DELETE route registration
- fastify.patch — PATCH route registration
- fastify.head — HEAD route registration
- fastify.options — OPTIONS route registration
- fastify.all — all-methods route registration
- fastify.register — plugin registration
- fastify.listen — start listening
- fastify.ready — wait for boot
- fastify.inject — in-process request injection (light-my-request)
- fastify.addHook — lifecycle hook registration
- fastify.decorate — add a decorated property
- fastify.addSchema — add a shared schema
- fastify.setErrorHandler — global error handler
- fastify.setNotFoundHandler — 404 handler
- fastify.after — plugin-sequencing after callback

## behavior
- fastify() -> buildRouting -> createServer — server bootstrap (fastify.js)
- route registration -> onRoute hooks -> router lookup — route registration flow (lib/route.js)
- request -> routing -> hooks chain -> handler -> reply — request lifecycle (lib/route.js)
- fastify.register(plugin) -> avvio boot -> plugin load — plugin loading (fastify.js)
- reply.send -> serialize -> write — response flow (lib/reply.js)
- validation -> schema -> 400 — schema validation flow (lib/validation.js)
- request -> 404 handler -> fourOhFour — 404 flow (lib/four-oh-four.js)
- decorate -> plugin scope override — decoration flow (lib/decorate.js)

## state_authority
- fastify — the instance state: routes, hooks, plugins, schemas (fastify.js)
- kState — the instance state bucket (fastify.js)
- Router — the routing state (lib/route.js)
- Context — per-route context state (lib/context.js)
- Reply — the response state (lib/reply.js)
- Request — the request state (lib/request.js)
- avvio — the plugin boot state (fastify.js)
- SchemaController — the schema store (lib/schema-controller.js)
- fourOhFour — the 404 state (lib/four-oh-four.js)

## contracts
- fastify.get("/", handler) — GET route contract
- fastify.post("/users", handler) — POST route contract
- fastify.put("/users/:id", handler) — PUT route contract
- fastify.delete("/users/:id", handler) — DELETE route contract
- fastify.register(plugin, opts) — plugin contract
- fastify.listen(port) — listen contract
- fastify.inject(opts) — injection contract
- fastify.addHook("onRequest", fn) — hook contract
- fastify.decorate("name", value) — decorate contract
- fastify.setErrorHandler(fn) — error handler contract
- fastify.setNotFoundHandler(fn) — 404 contract
- :id — route parameter contract
- schema: { body: ... } — schema contract
- reply.code(status) — status code contract
- reply.send(payload) — send contract

## landmarks
- fastify — the instance factory (fastify.js)
- buildRouting — the router builder (lib/route.js)
- findRoute — route lookup (lib/route.js)
- createServer — the server wrapper (lib/server.js)
- Reply — the reply class (lib/reply.js)
- Request — the request class (lib/request.js)
- SchemaController — the schema controller (lib/schema-controller.js)
- fourOhFour — the 404 handler (lib/four-oh-four.js)
- LogController — the logging controller (fastify.js)
- ContentTypeParser — the body parser (lib/content-type-parser.js)

## tests
- test/route.test.js — routing tests
- test/inject.test.js — injection tests
- test/hooks.test.js — hooks tests
- test/validation.test.js — validation tests
- test/schema.test.js — schema tests
- test/plugin.test.js — plugin tests
- test/404s.test.js — 404 tests
- test/reply.test.js — reply tests
- test/listen.test.js — listen tests
