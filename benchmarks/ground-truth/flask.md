# flask
> https://github.com/pallets/flask | Python | python service | ~18k LOC

## architecture
- Flask — the WSGI application class in src/flask/app.py (line 110), central registry of routes/config
- Blueprint — route-grouping class in src/flask/blueprints.py (line 18), registered via register_blueprint
- Scaffold — shared behavior of Flask and Blueprint in src/flask/sansio/scaffold.py (line 52), holds route decorators
- App — sansio base of the Flask class in src/flask/sansio/app.py (line 59)
- View — class-based view base in src/flask/views.py (line 16), exposes as_view
- SessionInterface — session backend contract in src/flask/sessions.py (line 100)
- Config — dict-based configuration object in src/flask/config.py (line 50)
- Environment — Jinja environment class in src/flask/templating.py (line 36)
- FlaskGroup — click Group class powering the flask CLI in src/flask/cli.py (line 531)

## entrypoints
- app.run — starts the development server (src/flask/app.py line 633)
- @app.route — URL-rule decorator for view functions (src/flask/sansio/scaffold.py line 344)
- @app.get — GET shortcut for route (src/flask/sansio/scaffold.py line 296)
- @app.post — POST shortcut for route (src/flask/sansio/scaffold.py line 312)
- register_blueprint — attaches a Blueprint's rules to the app (src/flask/sansio/app.py line 570)
- add_url_rule — low-level rule registration called by route (src/flask/sansio/app.py line 605)
- flask = "flask.cli:main" — console script in pyproject.toml (line 82)
- `flask run` — click.command("run") dev-server command (src/flask/cli.py line 882)
- locate_app — resolves the app import string to a Flask instance (src/flask/cli.py line 241)

## behavior
- full_dispatch_request — request pipeline: preprocess -> dispatch -> process response (src/flask/app.py line 995)
- dispatch_request — matches the URL rule and calls the view function (src/flask/app.py line 969)
- preprocess_request — runs before_request handlers (src/flask/app.py line 1369)
- process_response — runs after_request handlers and saves the session (src/flask/app.py line 1397)
- url_for — builds URLs from endpoints, resolved via app.url_map (src/flask/helpers.py line 200)
- render_template — renders a Jinja template with the app context (src/flask/templating.py line 136)
- find_best_app — CLI heuristic to discover the app in an imported module (src/flask/cli.py line 41)

## state_authority
- app.view_functions — dict mapping endpoint names to view functions (src/flask/sansio/scaffold.py line 108)
- self.url_map — werkzeug Map holding all URL rules, mutated by add_url_rule (src/flask/sansio/app.py line 402)
- session_interface — session backend instance, defaults to SecureCookieSessionInterface (src/flask/app.py line 253)
- jinja_env — cached Jinja Environment for template loading (src/flask/sansio/app.py line 467)
- app.config — Config instance holding configuration values (src/flask/config.py)
- flask.session — context-local proxies exported from src/flask/globals.py

## contracts
- `GET /` — route contract via @app.get("/") in tests/test_basic.py (line 111)
- `POST /` — route contract via @app.post("/") in tests/test_basic.py (line 238)
- `/page/<int:page>` — int-converter path rule in tests/test_blueprints.py (line 273)
- `/<company_id>` — dynamic segment with subdomain="<company_id>" in tests/test_testing.py (line 309)
- --host — flask run options with defaults 127.0.0.1:5000 (src/flask/cli.py lines 883-884)
- -A — CLI option selecting the app import path (src/flask/cli.py line 454)
- flask routes — CLI command listing registered rules with --sort/--all-methods (src/flask/cli.py line 1048)
- `GET /` — methods-array route contract (`@app.route("/", methods=["GET", "POST"])` in tests/test_basic.py line 34)

## landmarks
- MethodView — View dispatching HTTP methods to get/post/... methods in src/flask/views.py (line 138)
- SecureCookieSessionInterface — default signed-cookie session backend in src/flask/sessions.py (line 284)
- DispatchingJinjaLoader — Jinja loader searching app + blueprint template folders (src/flask/templating.py line 49)

## tests
- tests/test_basic.py — request dispatching, session and routing behavior
- tests/test_blueprints.py — blueprint route registration and URL building
- tests/test_cli.py — CLI command and option coverage
- tests/test_templating.py — render_template/render_template_string behavior
- tests/test_user_error_handler.py — error handler registration and dispatch
- tests/test_session_interface.py — session interface contract
- tests/test_views.py — View/MethodView behavior
