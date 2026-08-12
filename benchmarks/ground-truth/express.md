# express
> https://github.com/expressjs/express | TypeScript/JS | ts backend | ~21k LOC

## architecture
- `createApplication` — app factory in lib/express.js; returns the request-handling `app` function
- `app` — application prototype in lib/application.js; holds settings, router, and view engines
- `Router` — router constructor re-exported from the external `router` package in lib/express.js
- `Route` — `Router.Route` re-exported in lib/express.js; one route with stacked handlers
- `req` — request prototype in lib/request.js with lazy getters (query, protocol, ip, hostname)
- `res` — response prototype in lib/response.js with send/json/status/render methods
- `View` — template view class in lib/view.js that invokes the registered engine
- `json` — body-parser JSON middleware re-exported from lib/express.js
- `urlencoded` — body-parser urlencoded middleware re-exported from lib/express.js
- `static` — serve-static middleware re-exported from lib/express.js

## entrypoints
- `require('express')` — package entry index.js which just re-exports lib/express.js
- `app.listen` — boots the server via http.createServer(this) in lib/application.js
- `app.handle` — dispatches a req/res pair into the middleware pipeline
- `app.init` — initialization hook called by createApplication
- `app.engine` — registers a template engine callback for a file extension
- `app.param` — registers route-parameter middleware on the router
- `exports.application` — prototype export that apps subclass from

## behavior
- `app.use` — middleware registration proxied to the router: `app.use` -> `this.router.use` (application.js)
- `app.route` -> `router.route` — creates a chainable Route for a path
- `methods.forEach` -> `app.get` — per-verb handlers delegated to `router.VERB` at module load
- `app.listen` -> `http.createServer(this)` -> `app.handle` — full request-serving chain
- `app.render` -> `tryRender` -> `View.prototype.render` — view rendering pipeline
- `res.sendFile` -> `sendfile` helper — streams a file through the send package
- `res.redirect` — defaults to status 302 and sets the Location header (response.js)
- `res.json` — serializes with the `json escape` / `json spaces` settings applied

## state_authority
- `this.router` — lazily constructed Router owned by the app (getrouter getter in application.js)
- `this.settings` — app settings store (etag, query parser, trust proxy, view engine)
- `this.engines` — map of registered template engines
- `this.cache` — compiled view cache consulted by app.render
- `res.locals` — response-local template variables merged into render opts in response.js
- `req.app` — back-reference to the app instance on every request
- `trustProxyDefaultSymbol` — sentinel used for trust-proxy inheritance bookkeeping

## contracts
- `GET /tobi` — verb route registration pattern (`app.get('/tobi')`, test/app.head.js)
- `GET /post/:id` — path-param route with req.params.id (`app.get('/post/:id')`, test/app.param.js)
- `GET /user/:id{/:op}` — optional-group path syntax (`app.get('/user/:id{/:op}')`, test/req.route.js)
- `GET /` — route string asserted in test/app.router.js
- `case sensitive routing` — routing setting toggling Router case sensitivity
- `trust proxy` — proxy-trust setting compiled via compileTrust
- `view engine` — default template engine extension setting
- `query parser` — query parsing mode setting ('simple' default)
- `res.status(code)` — integer status setter that throws on non-integer codes

## landmarks
- `methods` — lowercased HTTP verb list derived from node:http METHODS in lib/utils.js
- `app.request` — per-app req/res prototypes created in createApplication

## tests
- test/app.router.js — core routing behavior suite
- test/app.param.js — param middleware and next('route') semantics
- test/app.head.js — HEAD request and send() detection
- test/app.options.js — automatic OPTIONS handling and Allow header
- test/res.json.js — JSON response format contracts
- test/req.ip.js — trusted-proxy IP resolution
- test/req.route.js — req.route.path reflection
- test/utils.js — shared request(app) supertest helper
