# httpx
> https://github.com/encode/httpx | Python | python http client lib | ~24k LOC

## architecture
- httpx — the package root: Client, AsyncClient, top-level request API (httpx/)
- _client.py — the client: BaseClient, Client, AsyncClient (httpx/_client.py)
- _models.py — request/response model: Request, Response, Headers, Cookies (httpx/_models.py)
- _transports — transport layer: HTTPTransport, AsyncHTTPTransport, ASGITransport, WSGITransport, MockTransport (httpx/_transports/)
- _auth.py — authentication: Auth, BasicAuth, DigestAuth, NetRCAuth (httpx/_auth.py)
- _config.py — client configuration: Timeout, Limits, Proxy, SSL context (httpx/_config.py)
- _decoders.py — content decoding: GZipDecoder, DeflateDecoder, BrotliDecoder, ZStandardDecoder (httpx/_decoders.py)
- _urls.py — URL parsing and joining (httpx/_urls.py)
- _main.py — the integrated command-line client (httpx/_main.py)
- _exceptions.py — the exception hierarchy (httpx/_exceptions.py)
- _status_codes.py — HTTP status code constants (httpx/_status_codes.py)
- docs — documentation site (docs/)

## entrypoints
- httpx.get — top-level sync GET helper
- httpx.post — top-level sync POST helper
- httpx.put — top-level sync PUT helper
- httpx.delete — top-level sync DELETE helper
- httpx.head — top-level sync HEAD helper
- httpx.patch — top-level sync PATCH helper
- httpx.options — top-level sync OPTIONS helper
- httpx.request — top-level sync request helper
- httpx.stream — top-level sync streaming helper
- httpx.AsyncClient — async client entry
- httpx.Client — sync client entry
- httpx.get_async — async GET helper (async_compatible)
- httpx.main — the CLI entry point (httpx/_main.py)
- httpx-cli — the `httpx` command-line client (console script)

## behavior
- BaseClient.send -> transport.handle_request -> Response — request lifecycle (httpx/_client.py)
- Request -> redirect loop -> follow_redirects — redirect following (httpx/_client.py)
- Client.get -> request -> send — top-level helper flow
- DigestAuth.auth_flow -> challenge/response — digest authentication (httpx/_auth.py)
- Response.iter_bytes -> decoder.decode -> content — response decoding (httpx/_decoders.py)
- Client.build_request -> Request -> auth flow — request building (httpx/_client.py)

## state_authority
- Client — connection pool, cookies, auth, base URL, headers (httpx/_client.py)
- BaseClient._state — UNINITIALIZED/OPENED/CLOSED client state (httpx/_client.py)
- ClientState — the client state enum (httpx/_client.py)
- Headers — the header store (httpx/_models.py)
- Cookies — the cookie jar (httpx/_models.py)
- Timeout — per-operation timeout configuration (httpx/_config.py)
- Limits — connection pool limits (httpx/_config.py)
- Proxy — proxy configuration state (httpx/_config.py)
- URL — parsed URL state (httpx/_urls.py)

## contracts
- httpx.get(url) — GET contract
- httpx.post(url, data=...) — POST contract
- httpx.put(url) — PUT contract
- httpx.patch(url) — PATCH contract
- httpx.delete(url) — DELETE contract
- httpx.request(method, url) — generic method contract
- Client.get(url) — client GET contract
- Client.send(request) — transport-level send contract
- AsyncClient.get(url) — async GET contract
- Transport.handle_request(request) — transport interface contract
- http:// — HTTP scheme contract
- https:// — HTTPS scheme contract
- HTTP/1.1 — protocol contract
- HTTP/2 — protocol contract

## landmarks
- BaseClient — the shared sync/async client base (httpx/_client.py)
- Client — sync client implementation (httpx/_client.py)
- AsyncClient — async client implementation (httpx/_client.py)
- Request — the request model (httpx/_models.py)
- Response — the response model (httpx/_models.py)
- Auth — auth base class (httpx/_auth.py)
- Timeout — timeout config (httpx/_config.py)
- HTTPTransport — the default transport (httpx/_transports/default.py)
- ASGITransport — ASGI in-process transport (httpx/_transports/asgi.py)
- WSGITransport — WSGI in-process transport (httpx/_transports/wsgi.py)
- MockTransport — handler-based test transport (httpx/_transports/mock.py)
- HTTPError — base exception (httpx/_exceptions.py)

## tests
- tests/test_client.py — client tests
- tests/test_models.py — request/response model tests
- tests/test_auth.py — auth tests
- tests/test_config.py — timeout/limits config tests
- tests/test_decoders.py — content decoder tests
- tests/test_transports.py — transport tests
- tests/test_main.py — CLI tests
- tests/test_urls.py — URL tests
