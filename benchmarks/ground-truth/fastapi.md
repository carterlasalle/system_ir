# fastapi
> https://github.com/fastapi/fastapi | Python | python service | ~113k LOC

## architecture
- FastAPI — the app class in fastapi/applications.py (line 42), core of the framework; note: no fastapi/main.py in this version
- APIRouter — route-grouping component in fastapi/routing.py (line 2255), included into FastAPI apps via include_router
- APIRoute — path-operation route class in fastapi/routing.py (line 1126), wraps endpoint + response model

## entrypoints
- `GET /items/{item_id}` — route decorator on a FastAPI instance, e.g. docs_src/app_testing/tutorial003_py310.py
- app.get — HTTP GET decorator method on FastAPI (fastapi/applications.py line 1646)
- app.post — HTTP POST decorator method on FastAPI (fastapi/applications.py line 2397)
- include_router — attaches an APIRouter's routes to the app (fastapi/applications.py line 1441)
- add_api_route — low-level route registration used by the decorators (fastapi/applications.py line 1165)
- websocket — websocket route decorator (fastapi/applications.py line 1376)
- exception_handler — registers custom exception handlers per status/class (fastapi/applications.py line 4729)
- openapi — method generating the app's OpenAPI schema (fastapi/applications.py line 1070)
- fastapi = "fastapi.cli:main" — CLI console script in pyproject.toml (line 120)

## behavior
- get_dependant — builds the Dependant tree from an endpoint signature (fastapi/dependencies/utils.py line 271)
- solve_dependencies — async resolution of the dependency tree into request args (fastapi/dependencies/utils.py line 586)
- get_request_handler — wraps endpoint + dependant into the ASGI handler (fastapi/routing.py line 375)
- request_response — converts a sync function into an ASGI app (fastapi/routing.py line 121)
- get_openapi_path — converts one APIRoute into an OpenAPI operation entry (fastapi/openapi/utils.py line 311)
- generate_operation_id — derives operationId strings like read_items_items__get (fastapi/openapi/utils.py line 266)
- jsonable_encoder — encodes response objects (datetimes, Decimals, models) to JSON-safe values (fastapi/encoders.py line 129)

## state_authority
- self.dependency_overrides — app-level dict of overridden dependencies (fastapi/applications.py line 967)
- self.router — the internal APIRouter holding all registered routes (fastapi/applications.py init)
- app.state — State object attached to the app instance (fastapi/applications.py line 966)
- model_name_map — per-app map of model names during OpenAPI generation (openapi/utils.py)
- app.openapi_url — URL where the generated schema is served (fastapi/applications.py setup)

## contracts
- `GET /items/{item_id}` — canonical path+method contract: path literal in docs_src/app_testing/tutorial003_py310.py, "get" method key in tests/test_openapi_separate_input_output_schemas.py
- `POST /items/` — create-item contract from docs_src/app_b_py310/main.py (@app.post("/items/"))
- `GET /` — root route contract from docs_src/advanced_middleware/tutorial001_py310.py
- `/path/{item_id}` — openapi path assertion in tests/test_application.py (line 104)
- `read_items_items__get` — operationId asserted in tests/test_openapi_separate_input_output_schemas.py (line 171)
- response_model=Item — response model contract in docs_src/additional_responses/tutorial001_py310.py
- `GET /items/{item_id}` — APIRouter route contract in tests/test_router_include_context.py (line 43)
- `Header` — header parameter contract (x_token: str = Header()) in docs_src/app_b_py310/main.py

## landmarks
- APIWebSocketRoute — websocket route class in fastapi/routing.py (line 801), subclasses Starlette WebSocketRoute
- Dependant — dependency-graph node dataclass in fastapi/dependencies/models.py (line 32)
- ParamTypes — enum of parameter locations (query/header/path/cookie) in fastapi/params.py (line 19)
- Path — path parameter class in fastapi/params.py (line 137), sets in_ = ParamTypes.path
- Query — query parameter class in fastapi/params.py (line 221), sets in_ = ParamTypes.query
- Body — request-body field class in fastapi/params.py (line 469), base of Form/File
- Depends — frozen dataclass declaring a dependency callable in fastapi/params.py (line 746)
- Security — Depends subclass with scopes in fastapi/params.py (line 753)
- jsonable_encoder — JSON-compatible object encoder in fastapi/encoders.py (line 129)
- get_openapi — OpenAPI schema generator in fastapi/openapi/utils.py (line 585)
- BackgroundTasks — background task container in fastapi/background.py with add_task (line 40)
- HTTPException — error type raised in endpoints, exported from fastapi/exceptions.py

## tests
- tests/test_application.py — openapi schema and route behavior assertions
- tests/test_dependencies_utils.py — dependency resolution unit tests
- tests/test_router_include_context.py — router include context with @router.get routes
- tests/test_openapi_separate_input_output_schemas.py — openapi operation assertions
- tests/test_fastapi_cli.py — CLI entrypoint tests
- docs_src/app_testing/tutorial003_py310.py — tutorial app used by docs test runner
- tests/test_dependency_overrides.py — dependency override behavior
