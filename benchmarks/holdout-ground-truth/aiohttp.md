# aiohttp
> https://github.com/aio-libs/aiohttp | Python | python service (lib) | ~137k LOC

## architecture
- aiohttp.web — top-level web module: run_app, web.Application, route decorators (aiohttp/web.py)
- Application — the web app container with router, middlewares, signals, subapps (aiohttp/web_app.py)
- UrlDispatcher — URL routing table: add_route, resources, named routes (aiohttp/web_urldispatcher.py)
- Request — incoming HTTP request (aiohttp/web_request.py)
- Response — outgoing HTTP response (aiohttp/web_response.py)
- StreamResponse — base streaming response class (aiohttp/web_response.py)
- ClientSession — HTTP client session with connector + cookie jar (aiohttp/client.py)
- ClientConnector — connection pooling transport (aiohttp/connector.py)
- web_middlewares — middleware chain support (aiohttp/web_middlewares.py)

## entrypoints
- web.run_app — serve an Application over a server (aiohttp/web.py)
- web.Application — app factory entrypoint
- app.router.add_route — manual route registration (aiohttp/web_urldispatcher.py)
- routes.get/post — @routes.get('/path') decorator route registration (aiohttp/web_routedef.py)
- app.add_routes — register a list of route definitions (aiohttp/web_app.py)
- ClientSession.request — per-session HTTP request (aiohttp/client.py)
- ClientSession.get/post — verb shortcuts (aiohttp/client.py)
- python -m aiohttp.web — CLI server entry (aiohttp/web.py main)
- web.WebSocketResponse — websocket endpoint entrypoint

## behavior
- handle_request -> UrlDispatcher.resolve -> handler call -> Response.prepare — request lifecycle (aiohttp/web_protocol.py)
- _build_middlewares -> handler chain — middleware wrapping (aiohttp/web_app.py)
- ClientSession._request -> connector.connect -> response.read — client request lifecycle (aiohttp/client.py)
- UrlDispatcher.resolve -> _find_match -> match_info — route matching (aiohttp/web_urldispatcher.py)
- WebSocketResponse.prepare -> websocket handshake — ws upgrade flow
- app._on_startup/_on_cleanup signal dispatch — app lifecycle signals (aiohttp/web_app.py)

## state_authority
- app.router — the app's routing table
- app.middlewares — app-level middleware stack (FrozenList)
- app['key'] — app-level storage dict (Application.__getitem__)
- ClientSession.connector — shared connection pool
- ClientSession.cookie_jar — persistent cookies across requests
- UrlDispatcher._resources — registered resource list (aiohttp/web_urldispatcher.py)
- Request.app — back-reference to owning Application (aiohttp/web_request.py)

## contracts
- GET /path — route pattern + method contract via add_route('GET', '/path', handler)
- web.run_app(app, host='0.0.0.0', port=8080) — server startup contract
- @routes.get('/items/{item_id}') — decorator route contract (aiohttp/web_routedef.py)
- --host / --port — python -m aiohttp.web CLI flags (aiohttp/web.py)
- '/items/{item_id}' — named path parameter contract (web_urldispatcher.py)
- '/download/*' — wildcard path match contract
- middleware(handler) -> handler — middleware signature contract (aiohttp/typedefs.py)
- WS message JSON/bytes frames — websocket frame contracts

## landmarks
- AbstractResource — resource base class (aiohttp/web_urldispatcher.py)
- AbstractRoute — route abstraction
- AppKey — typed app-storage key (aiohttp/helpers.py)
- FileResponse — file-serving response (aiohttp/web_fileresponse.py)
- ClientResponse — client-side response (aiohttp/client_reqrep.py)
- CleanupError — raised during app cleanup (aiohttp/web_app.py)
- HTTPException — exception base for HTTP errors (aiohttp/web_exceptions.py)
- Signal — app signal container (aiosignal)

## tests
- tests/ — the aiohttp test suite
- tests/test_web_application.py — app/router behavior
- tests/test_web_urldispatcher.py — route matching
- tests/test_client_session.py — client session behavior
- tests/test_web_functional.py — end-to-end server tests
