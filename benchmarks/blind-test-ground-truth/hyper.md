# hyper
> https://github.com/hyperium/hyper | Rust | rust http library | ~40k LOC

## architecture
- hyper — the library root (src/)
- src/client — the client: legacy Client, conn builders (src/client/)
- src/client/legacy — the legacy high-level client: Client, Builder (src/client/legacy/)
- src/client/conn — low-level connection: http1 Builder, http2 Builder, handshake (src/client/conn/)
- src/server — the server: conn, accept (src/server/)
- src/server/conn — connection serving: http1 Builder, http2 Builder, serve_connection (src/server/conn/)
- src/proto — the protocol implementations: h1, h2 (src/proto/)
- src/proto/h1 — the HTTP/1.1 codec: Dispatcher, role (src/proto/h1/)
- src/proto/h2 — the HTTP/2 codec (src/proto/h2/)
- src/service — the service traits: Service, HttpService (src/service/)
- src/body — the body abstraction: Body, Incoming, SizeHint (src/body/)
- src/ext — extension helpers (src/ext/)
- src/rt — runtime abstraction: Read, Write, Sleep, Timer (src/rt/)
- src/ffi — the C API bindings (src/ffi/)
- src/upgrade.rs — connection upgrade support (src/upgrade.rs)
- src/mock.rs — the mock connector (src/mock.rs)

## entrypoints
- hyper::Client — the client entry (legacy)
- hyper::client::conn::http1::handshake — low-level HTTP/1 client
- hyper::client::conn::http2::handshake — low-level HTTP/2 client
- hyper::client::conn::http1::Builder — HTTP/1 client builder
- hyper::client::conn::http2::Builder — HTTP/2 client builder
- hyper::Server — the server entry
- hyper::server::conn::http1::Builder — HTTP/1 server builder
- hyper::server::conn::http2::Builder — HTTP/2 server builder
- Builder::serve_connection — serve a connection
- Client::get — GET request
- Client::request — generic request
- Client::builder — client builder entry
- Server::from_tcp — serve on a TCP listener
- hyper::service::service_fn — service from a function
- hyper::body::to_bytes — read the body to bytes
- hyper::Body — the body entry
- hyper::Request — request type
- hyper::Response — response type
- hyper::rt::Executor — runtime executor trait

## behavior
- Client::get -> connect -> request -> response — client request flow (legacy)
- handshake -> connection -> dispatch — connection establishment (conn/)
- Builder::serve_connection -> h1 dispatcher -> response — server connection flow (server/conn/http1.rs)
- service_fn -> call(request) -> response — service invocation (service/)
- Request -> h1 codec encode -> write — request serialization (proto/h1/)
- response -> h1 dispatcher -> decode -> body — response deserialization (proto/h1/)
- Connection::poll -> drive -> complete — connection lifecycle (conn/)
- Body::poll_frame -> frame stream (body/)

## state_authority
- Client — the client state: connector, executor, builder config (legacy/client.rs)
- Connection — the connection state (conn/)
- SendRequest — the request sender state (conn/)
- Dispatcher — the h1 dispatch state (proto/h1/dispatch.rs)
- Body — the body stream state (body/)
- Builder — the server builder state (server/conn/http1.rs)
- Server — the accept-loop state (server/mod.rs)
- Executor — the runtime executor state (rt/)
- Upgrade — the upgrade state (upgrade.rs)

## contracts
- http:// — HTTP scheme contract
- https:// — HTTPS scheme contract
- GET /path HTTP/1.1 — HTTP/1.1 request contract
- GET /path HTTP/2 — HTTP/2 request contract
- 200 OK — success status contract
- 404 Not Found — not-found status contract
- 500 Internal Server Error — error status contract
- Content-Type: text/plain — header contract
- Connection: upgrade — upgrade contract
- hyper::Client::builder().build(connector) — client build contract
- client.get(uri) — get contract
- client.request(request) — request contract
- serve_connection(io, service) — serve contract
- Server::from_tcp(listener) — from_tcp contract
- service_fn(|req| async { Ok(resp) }) — service contract
- Connection::with_upgrades — upgrade support contract

## landmarks
- Client — the legacy client (client/legacy/client.rs)
- Builder — the client builder (client/legacy/client.rs)
- Server — the server (server/mod.rs)
- Connection — the connection type (conn/)
- SendRequest — the request sender (conn/)
- Body — the body type (body/)
- Incoming — the incoming body (body/incoming.rs)
- SizeHint — the body size hint (body/size_hint.rs)
- Dispatcher — the h1 dispatcher (proto/h1/dispatch.rs)
- service_fn — the service adapter (service/mod.rs)
- Request — the request type (lib.rs)
- Response — the response type (lib.rs)
- Error — the error type (error.rs)
- Upgrade — the upgrade type (upgrade.rs)

## tests
- src/client/tests.rs — client tests
- src/server/tests.rs — server tests
- src/proto/h1/tests/ — h1 protocol tests
- src/proto/h2/tests/ — h2 protocol tests
- src/service/tests.rs — service tests
- src/ffi/tests/ — ffi tests
- tests/ — integration tests
