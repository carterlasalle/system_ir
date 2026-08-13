# axum
> https://github.com/tokio-rs/axum | Rust | rust web framework | ~40k LOC

## architecture
- axum — the framework root crate (axum/)
- axum-core — the core traits: FromRequestParts, IntoResponseParts (axum-core/)
- axum-macros — the derive macros (axum-macros/)
- axum-extra — the extra utilities crate (axum-extra/)
- src/routing — routing: Router, MethodRouter, MethodFilter (axum/src/routing/)
- src/handler — the handler machinery (axum/src/handler/)
- src/extract — the extractors: State, Query, Path, Json, Form, Extension, Multipart (axum/src/extract/)
- src/response — the response types: IntoResponse, Html, Redirect (axum/src/response/)
- src/middleware — middleware helpers: from_fn, from_extractor, map_request, map_response (axum/src/middleware/)
- src/serve — the server integration: serve, with_graceful_shutdown (axum/src/serve/)
- src/json.rs — the Json extractor (axum/src/json.rs)
- src/form.rs — the Form extractor (axum/src/form.rs)
- src/body — the body types: Body, BoxBody (axum/src/body/)

## entrypoints
- Router::new — the router entry
- Router::route — route registration
- Router::nest — nested router mounting
- Router::merge — router merging
- Router::layer — middleware application
- Router::fallback — fallback handler
- Router::with_state — state attachment
- axum::serve — serve on a listener
- Serve::with_graceful_shutdown — graceful shutdown
- MethodRouter::on — method-scoped handler
- MethodRouter::get — GET handler
- MethodRouter::post — POST handler
- MethodRouter::put — PUT handler
- MethodRouter::delete — DELETE handler
- MethodRouter::any — any-method handler
- Router::into_make_service — service conversion
- axum::extract::State — state extractor
- axum::extract::Query — query extractor
- axum::extract::Path — path extractor
- axum::Json — json extractor/response

## behavior
- Router::route -> routing tree insert -> request match (routing/mod.rs)
- serve(listener) -> accept loop -> dispatch to router (serve/mod.rs)
- request -> extractors -> handler -> IntoResponse (handler/)
- State extractor -> state injection -> handler (extract/state.rs)
- middleware layer -> next.run(request) -> response (middleware/)
- nest("/api", router) -> path stripping -> sub-router (routing/mod.rs)
- fallback handler -> unmatched route (routing/mod.rs)
- with_graceful_shutdown -> signal -> shutdown (serve/mod.rs)

## state_authority
- Router — the routing state: routes, fallback, state, layers (routing/mod.rs)
- MethodRouter — the method routing state (routing/method_routing.rs)
- MethodFilter — the method filter state (routing/method_routing.rs)
- State — the application state (extract/state.rs)
- Serve — the server state (serve/mod.rs)
- Extension — the extension store (extract/extension.rs)
- Body — the body state (body/)
- RouterState — the state type parameter

## contracts
- Router::new().route("/users", get(handler)) — GET route contract
- Router::new().route("/users/:id", get(handler)) — parameterized route contract
- .route("/", post(handler)) — POST route contract
- .route("/x", put(handler)) — PUT route contract
- .route("/x", delete(handler)) — DELETE route contract
- .nest("/api", sub_router) — nest contract
- .merge(other_router) — merge contract
- .layer(middleware) — layer contract
- .fallback(handler) — fallback contract
- .with_state(state) — state contract
- axum::serve(listener, router) — serve contract
- .with_graceful_shutdown(signal) — graceful shutdown contract
- State<T> — state extractor contract
- Query<T> — query extractor contract
- Path<T> — path extractor contract
- Json<T> — json extractor contract
- Form<T> — form extractor contract
- Extension<T> — extension extractor contract
- :id — path parameter contract

## landmarks
- Router — the router struct (routing/mod.rs)
- MethodRouter — the method router (routing/method_routing.rs)
- MethodFilter — the filter enum (routing/method_routing.rs)
- serve — the serve function (serve/mod.rs)
- State — the state extractor (extract/state.rs)
- Query — the query extractor (extract/query.rs)
- Path — the path extractor (extract/path/)
- Json — the json extractor (json.rs)
- Form — the form extractor (form.rs)
- Extension — the extension extractor (extract/extension.rs)
- IntoResponse — the response trait (response/mod.rs)
- FromRequestParts — the request trait (extract/mod.rs)
- Handler — the handler trait (handler/mod.rs)
- Body — the body type (body/mod.rs)

## tests
- axum/src/routing/tests/ — routing tests
- axum/src/extract/tests.rs — extractor tests
- axum/src/handler/tests.rs — handler tests
- axum/src/response/tests.rs — response tests
- axum/tests/ — integration tests
- axum/src/serve/tests/ — serve tests
