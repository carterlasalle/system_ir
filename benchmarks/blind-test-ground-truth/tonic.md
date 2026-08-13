# tonic
> https://github.com/hyperium/tonic | Rust | rust grpc framework | ~80k LOC

## architecture
- tonic — the main crate: gRPC client/server on hyper (tonic/tonic/src/)
- tonic/src/client — the client: Grpc, service (tonic/src/client/)
- tonic/src/server — the server: Grpc, service, NamedService (tonic/src/server/)
- tonic/src/transport — the transport: Channel, Endpoint, Server (tonic/src/transport/)
- tonic/src/transport/channel — the channel: Channel, Endpoint, connect (tonic/src/transport/channel/)
- tonic/src/transport/server — the server transport: Server, Router (tonic/src/transport/server/)
- tonic/src/codec — the codecs: Codec, DecodeBuf, EncodeBuf (tonic/src/codec/)
- tonic/src/metadata — the metadata map: MetadataMap, MetadataValue (tonic/src/metadata/)
- tonic/src/status.rs — the Status type (tonic/src/status.rs)
- tonic/src/request.rs — the Request type (tonic/src/request.rs)
- tonic/src/response.rs — the Response type (tonic/src/response.rs)
- tonic/src/body.rs — the body types (tonic/src/body.rs)
- tonic/src/transport/tls.rs — TLS configuration (tonic/src/transport/tls.rs)
- tonic-build — the protobuf codegen crate (tonic-build/)
- tonic-prost — the prost integration (tonic-prost/)
- examples — example services (examples/)

## entrypoints
- tonic::transport::Channel — the client channel entry
- Endpoint::from_static — endpoint from a static URL
- Endpoint::from_shared — endpoint from a shared URL
- Channel::connect — async channel connection
- Endpoint::connect_lazy — lazy channel connection
- tonic::transport::Server::builder — the server builder
- Server::add_service — service registration
- Server::serve — serve on a socket address
- Router::add_service — service registration on a router
- tonic::server::Grpc — the grpc service wrapper
- tonic::client::Grpc — the grpc client wrapper
- tonic::Request — the request entry
- tonic::Response — the response entry
- tonic::Status — the status entry
- tonic::metadata::MetadataMap — the metadata entry
- tonic::codegen — codegen re-exports for generated code

## behavior
- Client::unary -> Grpc::unary -> channel -> request (client/grpc.rs)
- Server::serve -> accept -> hyper conn -> Grpc dispatch (transport/server/)
- add_service -> Router -> service dispatch table (transport/server/mod.rs)
- Endpoint::connect -> DNS resolve -> connection pool (transport/channel/)
- request -> codec encode -> h2 stream -> response (codec/)
- Status::from_grpc_code -> grpc status conversion (status.rs)
- Interceptor -> request interception (server/grpc.rs)
- MetadataMap insertion -> header metadata (metadata/)

## state_authority
- Channel — the connection state: endpoints, pool (transport/channel/mod.rs)
- Endpoint — the endpoint state: uri, timeouts, tls config (transport/channel/endpoint.rs)
- Server — the server state: services, tls, concurrency (transport/server/mod.rs)
- Router — the routing state (transport/server/mod.rs)
- Grpc — the service state (server/grpc.rs)
- Request — the request state: message, metadata, extensions (request.rs)
- Response — the response state (response.rs)
- Status — the grpc status state (status.rs)
- MetadataMap — the metadata state (metadata/)

## contracts
- grpc:// — grpc URL scheme
- https:// — https URL scheme
- Server::builder().add_service(svc).serve(addr) — serve contract
- Endpoint::from_static("http://127.0.0.1:50051") — endpoint contract
- Channel::connect(endpoint) — connect contract
- Grpc::unary(req) — unary call contract
- Grpc::server_streaming(req) — server streaming contract
- Grpc::client_streaming(req) — client streaming contract
- Grpc::streaming(req) — bidi streaming contract
- .with_interceptor(fn) — interceptor contract
- .timeout(duration) — timeout contract
- .tls_config(config) — tls contract
- .accept_http1(true) — http1 acceptance contract
- .concurrency_limit_per_connection(n) — concurrency contract
- Status::ok() — ok status contract
- Status::not_found("...") — not found status contract
- Status::internal("...") — internal error contract

## landmarks
- Channel — the channel (transport/channel/mod.rs)
- Endpoint — the endpoint (transport/channel/endpoint.rs)
- Server — the server (transport/server/mod.rs)
- Router — the router (transport/server/mod.rs)
- Grpc — the grpc service (server/grpc.rs)
- Grpc — the grpc client (client/grpc.rs)
- Codec — the codec trait (codec/mod.rs)
- MetadataMap — the metadata map (metadata/mod.rs)
- Status — the status type (status.rs)
- Request — the request type (request.rs)
- Response — the response type (response.rs)
- NamedService — the service name trait (server/mod.rs)
- Interceptor — the interceptor trait (server/grpc.rs)
- ClientTlsConfig — the client tls config (transport/tls.rs)

## tests
- tonic/tests/ — integration tests
- tonic/src/client/tests.rs — client tests
- tonic/src/server/tests.rs — server tests
- tonic/src/transport/tests.rs — transport tests
- tonic/src/codec/tests.rs — codec tests
- tonic/src/status/tests.rs — status tests
- tonic-build/tests/ — codegen tests
