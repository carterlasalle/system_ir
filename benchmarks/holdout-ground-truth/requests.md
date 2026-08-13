# requests
> https://github.com/psf/requests | Python | python lib | ~31k LOC

## architecture
- requests.api — module-level convenience functions (request/get/post/put/delete/head/patch/options) (src/requests/api.py)
- Session — stateful request engine persisting cookies/auth/proxies (src/requests/sessions.py)
- Request — outgoing request model (src/requests/models.py)
- PreparedRequest — fully materialized request (headers, URL, body) (src/requests/models.py)
- Response — incoming response model (status, headers, content) (src/requests/models.py)
- HTTPAdapter — urllib3-backed transport adapter (src/requests/adapters.py)
- RequestsCookieJar — cookie store (src/requests/cookies.py)
- hooks — request/response hook dispatch (src/requests/hooks.py)

## entrypoints
- requests.request — generic verb entrypoint (src/requests/api.py)
- requests.get — GET convenience (src/requests/api.py)
- requests.post — POST convenience (src/requests/api.py)
- requests.put — PUT convenience (src/requests/api.py)
- requests.delete — DELETE convenience (src/requests/api.py)
- requests.head — HEAD convenience (src/requests/api.py)
- requests.patch — PATCH convenience (src/requests/api.py)
- Session.request — per-session request dispatch (src/requests/sessions.py)
- Session.send — send a prepared request over the transport (src/requests/sessions.py)
- Session.get/post — session-level verb shortcuts (src/requests/sessions.py)

## behavior
- Session.request -> Session.prepare_request -> Session.send -> HTTPAdapter.send — full request lifecycle
- HTTPAdapter.send -> urllib3.urlopen -> build Response — transport round trip (src/requests/adapters.py)
- Session.send -> resolve_redirects — redirect following loop (src/requests/sessions.py)
- Response.raise_for_status -> HTTPError — error surfacing on 4xx/5xx (src/requests/models.py)
- Session.prepare_request -> PreparedRequest.prepare — request materialization (headers/auth/cookies)
- merge_setting — request/session setting merge precedence (src/requests/sessions.py)
- extract_cookies_to_jar -> merge_cookies — cookie persistence after response (src/requests/cookies.py)

## state_authority
- Session.cookies — persistent cookie jar per session
- Session.headers — session-level default headers (CaseInsensitiveDict)
- Session.auth — session-level auth default
- Session.proxies — session-level proxy mapping
- Response.content — decoded response body cache (src/requests/models.py)
- PreparedRequest.url/headers/body — fully resolved request state
- default_headers — library-wide default header set (src/requests/utils.py)

## contracts
- get(url, params=..., headers=..., timeout=...) — GET contract with keyword params (src/requests/api.py)
- timeout=(connect, read) — two-tuple timeout contract
- allow_redirects=False — disable redirect following
- stream=True — stream response body instead of buffering
- verify=False — skip TLS verification
- auth=('user', 'pass') — basic auth contract
- json={'key': 'value'} — JSON body contract
- files={'file': open(...)} — multipart upload contract
- proxies={'https': 'http://proxy:8080'} — proxy contract
- codes — status-code name mapping (src/requests/status_codes.py)

## landmarks
- HTTPBasicAuth — basic auth implementation (src/requests/auth.py)
- CaseInsensitiveDict — case-insensitive header mapping (src/requests/structures.py)
- merge_setting — per-key setting merge (src/requests/sessions.py)
- dispatch_hook — hook chain invocation (src/requests/hooks.py)
- default_hooks — the standard hook registry
- requote_uri — URL re-quoting helper (src/requests/utils.py)
- REDIRECT_STATI — redirect-eligible status set (src/requests/models.py)
- HTTPError — raised on 4xx/5xx responses (src/requests/exceptions.py)

## tests
- tests/ — the requests test suite (tests/test_requests.py, tests/test_sessions.py, etc.)
- tests/test_requests.py — end-to-end request behavior
- tests/test_sessions.py — session state and cookies
- tests/test_utils.py — utility functions
