#!/usr/bin/env python3
"""Behavioral evaluators for the writable coding benchmark.

Each task is decided by running the edited code (or a thin harness around
it) and asserting observable behavior. String/grep detectors are forbidden:
a comment-only patch MUST fail every evaluator.

Exit 0 = task success; nonzero = fail. Designed to run with cwd = the
disposable edited repo copy. Invoked as:

    python3 run.py <task-id>
"""
from __future__ import annotations

import ast
import importlib.util
import json
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import textwrap
from pathlib import Path

ROOT = Path.cwd()


def stub_module(name: str, **attrs):
    import types
    mod = types.ModuleType(name)
    for k, v in attrs.items():
        setattr(mod, k, v)
    sys.modules[name] = mod
    return mod


def stub_fastapi() -> None:
    class FastAPI:
        def get(self, *a, **k):
            return lambda fn: fn
        def post(self, *a, **k):
            return lambda fn: fn
        def delete(self, *a, **k):
            return lambda fn: fn
        def put(self, *a, **k):
            return lambda fn: fn
    stub_module("fastapi", FastAPI=FastAPI)


def stub_tenacity() -> None:
    def retry(*a, **k):
        def deco(fn):
            return fn
        return deco
    stub_module("tenacity", retry=retry, wait_fixed=lambda *a, **k: None, stop_after_attempt=lambda *a, **k: None)


def fail(msg: str) -> None:
    sys.stderr.write(msg.rstrip() + "\n")
    raise SystemExit(1)


def ok() -> None:
    raise SystemExit(0)


def load_py(rel: str, name: str):
    path = ROOT / rel
    if not path.is_file():
        fail(f"missing {rel}")
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def strip_comments_py(src: str) -> str:
    try:
        tree = ast.parse(src)
    except SyntaxError as e:
        fail(f"python parse error: {e}")
    lines = src.splitlines(True)
    for node in ast.walk(tree):
        if isinstance(node, ast.Expr) and isinstance(getattr(node, "value", None), ast.Constant):
            if isinstance(node.value.value, str):
                # docstring / standalone string — not a statement
                pass
    # Drop # comments via tokenize
    import io
    import tokenize
    out = []
    try:
        for tok in tokenize.generate_tokens(io.StringIO(src).readline):
            if tok.type == tokenize.COMMENT:
                continue
            out.append(tok)
    except tokenize.TokenError:
        return src
    return tokenize.untokenize(out)


def strip_comments_ts(src: str) -> str:
    src = re.sub(r"/\*[\s\S]*?\*/", "", src)
    src = re.sub(r"//.*?$", "", src, flags=re.M)
    return src


def run_node(js: str, timeout: int = 20) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["node", "--input-type=commonjs", "-e", js],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def node_assert(js: str, msg: str) -> None:
    proc = run_node(js)
    if proc.returncode != 0:
        fail(f"{msg}: {proc.stderr.strip() or proc.stdout.strip() or 'node exit '+str(proc.returncode)}")


def strip_ts_imports(src: str) -> str:
    """Drop import/export/parameter-property keywords; leave types for Node's strip-types."""
    src = re.sub(r"/\*[\s\S]*?\*/", "", src)
    src = re.sub(r"import\s+(?:type\s+)?[\s\S]*?from\s+['\"][^'\"]+['\"]\s*;?", "", src)
    src = re.sub(r"^export\s+default\s+", "", src, flags=re.M)
    src = re.sub(r"^export\s+", "", src, flags=re.M)
    src = re.sub(r"\b(private|public|protected|readonly)\s+", "", src)
    return src


def run_ts_harness(source_rel: str, prelude: str, body: str, msg: str) -> None:
    """Execute fixture TypeScript with mocks via `node --experimental-strip-types`.

    Import-stripping + eval() mangles `message: { value: string }` and
    `Promise<Array<{...}>>`. Node 22+ type stripping keeps a real parse.
    """
    src = strip_ts_imports((ROOT / source_rel).read_text())
    text = f"{prelude}\n{src}\n{body}\n"
    fd, path = tempfile.mkstemp(suffix=".ts", prefix="scc-eval-")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(text)
        proc = None
        last_err = ""
        for argv in (
            ["node", "--experimental-strip-types", "--no-warnings", path],
            ["node", "--no-warnings", path],
        ):
            proc = subprocess.run(
                argv,
                cwd=str(ROOT),
                capture_output=True,
                text=True,
                timeout=20,
            )
            if proc.returncode == 0:
                break
            last_err = proc.stderr.strip() or proc.stdout.strip() or f"node exit {proc.returncode}"
            if "bad option" not in last_err and "unknown option" not in last_err.lower():
                break
        if proc is None or proc.returncode != 0:
            fail(f"{msg}: {last_err}")
    finally:
        try:
            os.unlink(path)
        except OSError:
            pass


def run_ts_capture(source_rel: str, prelude: str, body: str, msg: str) -> str:
    """Like run_ts_harness, but return stdout on success."""
    src = strip_ts_imports((ROOT / source_rel).read_text())
    text = f"{prelude}\n{src}\n{body}\n"
    fd, path = tempfile.mkstemp(suffix=".ts", prefix="scc-eval-")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(text)
        proc = None
        last_err = ""
        for argv in (
            ["node", "--experimental-strip-types", "--no-warnings", path],
            ["node", "--no-warnings", path],
        ):
            proc = subprocess.run(
                argv,
                cwd=str(ROOT),
                capture_output=True,
                text=True,
                timeout=20,
            )
            if proc.returncode == 0:
                return proc.stdout or ""
            last_err = proc.stderr.strip() or proc.stdout.strip() or f"node exit {proc.returncode}"
            if "bad option" not in last_err and "unknown option" not in last_err.lower():
                break
        fail(f"{msg}: {last_err}")
    finally:
        try:
            os.unlink(path)
        except OSError:
            pass
    return ""


def _json_line(stdout: str) -> dict:
    for line in stdout.splitlines():
        if line.startswith("JSON:"):
            return json.loads(line[5:])
    fail("handler did not print JSON")
    return {}


# ---------------------------------------------------------------------------
# Scoped six (P0 empirical: comment-only must fail)
# ---------------------------------------------------------------------------

def eval_http_service_rename_transcript_field() -> None:
    # Stub the repository so the handler's RESPONSE SHAPE is what we assert,
    # not sqlite wiring. FastAPI is imported at module level; stub it so the
    # evaluator is hermetic.
    import types
    stub_fastapi()
    services = types.ModuleType("services")
    transcripts = types.ModuleType("services.transcripts")

    class TranscriptRepository:
        def find_all(self):
            return [{"id": 1, "transcript": "hello", "raw_text": "hello"}]

    class Normalizer:
        pass

    transcripts.TranscriptRepository = TranscriptRepository
    transcripts.Normalizer = Normalizer
    sys.modules["services"] = services
    sys.modules["services.transcripts"] = transcripts
    main = load_py("main.py", "main_eval")
    if not hasattr(main, "handle_transcripts"):
        fail("handle_transcripts missing")
    rows = main.handle_transcripts()
    if not isinstance(rows, list) or not rows:
        fail(f"handler did not return rows: {rows!r}")
    row = rows[0]
    if not isinstance(row, dict):
        fail(f"row is not a dict: {row!r}")
    keys = set(row.keys())
    has_new = "transcriptText" in keys or "transcript_text" in keys
    if not has_new:
        fail(f"response is missing transcriptText: {keys}")
    if "transcript" in keys:
        fail(f"old transcript key still present: {keys}")
    ok()


def eval_http_service_health_check() -> None:
    stub_fastapi()
    import types
    services = types.ModuleType("services")
    transcripts = types.ModuleType("services.transcripts")
    class TranscriptRepository:
        def find_all(self):
            return []
    class Normalizer:
        pass
    transcripts.TranscriptRepository = TranscriptRepository
    transcripts.Normalizer = Normalizer
    sys.modules["services"] = services
    sys.modules["services.transcripts"] = transcripts
    main = load_py("main.py", "main_health")
    if not hasattr(main, "health"):
        fail("health() missing")
    payload = main.health()
    if not isinstance(payload, dict):
        fail(f"health() must return a dict, got {type(payload)}")
    blob = {str(k).lower() for k in payload}
    if not any(k in blob for k in ("uptime", "checks", "dependencies", "ready")):
        fail(f"health payload has no readiness fields: {payload}")
    ok()


def eval_http_service_transcript_normalization() -> None:
    mod = load_py("services/transcripts.py", "transcripts_eval")
    class Mutating:
        def resolve(self, text):
            return "CHANGED-" + text
    n = mod.Normalizer(resolver=Mutating())
    out = n.normalize("raw audio text")
    if out != "raw audio text":
        fail(f"normalize must preserve raw text even when a resolver mutates, got {out!r}")
    ok()


def eval_py_queue_empty_messages() -> None:
    # Empty/blank messages must not be dispatched to process_incident.
    consumer_src = (ROOT / "consumer.py").read_text()
    # Install a processor that flags dispatch.
    import types
    called = {"n": 0}
    processor = types.ModuleType("processor")
    def process_incident(message, store=None):
        called["n"] += 1
        raise AssertionError("process_incident should not run for empty messages")
    processor.process_incident = process_incident
    sys.modules["processor"] = processor
    store_mod = types.ModuleType("store")
    class IncidentStore:
        def save_incident(self, *a, **k):
            fail("store.save_incident called for empty message")
    store_mod.IncidentStore = IncidentStore
    sys.modules["store"] = store_mod
    spec = importlib.util.spec_from_file_location("consumer_eval", ROOT / "consumer.py")
    consumer = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(consumer)
    for empty in ({}, {"text": ""}, {"text": "   "}, {"message": ""}):
        try:
            consumer.consume(empty)
        except Exception as e:
            fail(f"consume({empty!r}) raised: {e}")
    if called["n"] != 0:
        fail("empty messages were dispatched")
    # A real message must still dispatch (don't accept a consume() no-op).
    try:
        consumer.consume({"text": "smoke reported"})
        fail("real messages must still be dispatched")
    except AssertionError:
        pass
    ok()


def eval_py_queue_classification_fallback() -> None:
    # Fallback must save an incident when classify always raises, and must
    # record that classification did not succeed (classified=False or a
    # dedicated fallback field). The fixture already uses severity="unknown"
    # — require an explicit classified/fallback flag so a no-op cannot pass.
    stub_tenacity()
    store_mod = load_py("store.py", "store_eval")
    proc = load_py("processor.py", "processor_eval")
    saved = []
    class Capture(store_mod.IncidentStore):
        def __init__(self):
            pass
        def save_incident(self, *args, **kwargs):
            saved.append((args, kwargs))
    store = Capture()
    proc.process_incident({"text": "loud noise"}, store)
    if not saved:
        fail("fallback did not save an incident")
    args, kwargs = saved[0]
    blob = " ".join(str(a) for a in args) + " " + str(kwargs)
    # Require a signal that this was a fallback, not a successful classify.
    if "unclassified" not in blob.lower() and "classified" not in str(kwargs).lower() and "fallback" not in blob.lower():
        # Also accept a dict/kw with classified=False
        classified = kwargs.get("classified")
        if classified is not False and not (len(args) >= 3 and args[2] in (False, "fallback", "unclassified")):
            fail(f"fallback save must mark classification failure, got args={args} kwargs={kwargs}")
    ok()


def _invoke_save_incident(store_obj) -> None:
    """Call save_incident across payload/timestamp/created_at signatures."""
    import inspect
    candidates: list[tuple[tuple, dict]] = []
    extras = {
        "created_at": "now",
        "payload": {"ctx": 1},
        "timestamp": "now",
        "extra": {"ctx": 1},
        "context": {"ctx": 1},
        "metadata": {"ctx": 1},
    }
    try:
        sig = inspect.signature(store_obj.save_incident)
        args: list = []
        kwargs: dict = {}
        for name, param in sig.parameters.items():
            if name == "self":
                continue
            if param.kind in (inspect.Parameter.VAR_POSITIONAL, inspect.Parameter.VAR_KEYWORD):
                continue
            if name == "text":
                if not args:
                    args.append("hello")
                else:
                    kwargs["text"] = "hello"
            elif name == "severity":
                if len(args) == 1:
                    args.append("high")
                else:
                    kwargs["severity"] = "high"
            elif name in extras:
                kwargs[name] = extras[name]
            else:
                kwargs[name] = extras.get(name, "now")
        candidates.append((tuple(args), kwargs))
    except (TypeError, ValueError):
        pass
    candidates.extend([
        (("hello", "high"), {}),
        (("hello", "high"), {"created_at": "now"}),
        (("hello", "high"), {"payload": {"ctx": 1}}),
        (("hello", "high"), {"timestamp": "now"}),
        (("hello", "high", {"ctx": 1}), {}),
        (("hello", "high", "now"), {}),
    ])
    last = None
    for args, kwargs in candidates:
        try:
            store_obj.save_incident(*args, **kwargs)
            return
        except TypeError as exc:
            last = exc
            continue
    fail(f"save_incident could not be invoked: {last}")


def eval_py_queue_store_changes() -> None:
    mem = sqlite3.connect(":memory:")
    orig_connect = sqlite3.connect
    sqlite3.connect = lambda *a, **k: mem
    try:
        store = load_py("store.py", "store_chg")
        s = store.IncidentStore()
        _invoke_save_incident(s)
        info = list(mem.execute("PRAGMA table_info(incidents)"))
        if not info:
            fail("incidents table missing")
        cols = [r[1] for r in info]
        row = mem.execute("SELECT * FROM incidents").fetchone()
        if row is None:
            fail("save_incident persisted no row")
        data = dict(zip(cols, row))
        extras = {
            k: v for k, v in data.items()
            if k not in ("id", "text", "severity") and v not in (None, "")
        }
        if not extras:
            fail(f"save_incident did not persist extra populated columns: {data}")
        ok()
    except SystemExit:
        raise
    except Exception as e:
        fail(f"store.py failed: {e}")
    finally:
        sqlite3.connect = orig_connect
        mem.close()


def eval_queue_worker_asr_retry() -> None:
    ingest = ROOT / "src/ingest.ts"
    if not ingest.is_file():
        fail("src/ingest.ts missing")
    run_ts_harness(
        "src/ingest.ts",
        textwrap.dedent(
            """
            let calls = 0;
            async function transcribe(audio: unknown) {
              calls += 1;
              if (calls < 2) throw new Error('asr fail');
              return 'ok';
            }
            function normalize(raw: string) { return raw; }
            const redis = { set: async () => {}, publish: async () => {}, subscribe() {} };
            """
        ),
        textwrap.dedent(
            """
            if (typeof consume !== 'function') { console.error('consume missing'); process.exit(1); }
            consume({ value: 'x' }).then(() => {
              if (calls < 2) { console.error('transcribe calls=' + calls + ' (need retry)'); process.exit(1); }
              process.exit(0);
            }).catch((e: unknown) => { console.error(String(e)); process.exit(1); });
            """
        ),
        "asr retry behavior",
    )
    ok()


def eval_queue_worker_street_vocabulary() -> None:
    run_ts_harness(
        "src/geo/resolver.ts",
        "",
        textwrap.dedent(
            """
            if (typeof resolveStreetName !== 'function') { console.error('missing resolveStreetName'); process.exit(1); }
            const out = resolveStreetName('Main St');
            if (typeof out !== 'string') { console.error('not a string'); process.exit(1); }
            if (out === 'Main St') { console.error('identity: vocabulary not applied'); process.exit(1); }
            process.exit(0);
            """
        ),
        "street vocabulary must change the resolved name",
    )
    ok()


def eval_queue_worker_redis_persist() -> None:
    run_ts_harness(
        "src/ingest.ts",
        textwrap.dedent(
            """
            const sets: unknown[] = [];
            async function transcribe(audio: unknown) { return 'RAWTEXT'; }
            function normalize(raw: string) { return 'NORMTEXT'; }
            const redis = {
              set: async (k: unknown, v: unknown) => { sets.push([k, v]); },
              publish: async () => {},
              subscribe() {},
            };
            """
        ),
        textwrap.dedent(
            """
            consume({ value: 'x', topic: 'radio' }).then(() => {
              const persistedRaw = sets.some((s) => String((s as unknown[])[1]).includes('RAWTEXT'));
              const persistedNorm = sets.some((s) => String((s as unknown[])[1]).includes('NORMTEXT'));
              if (!(persistedRaw && persistedNorm)) {
                console.error('must persist BOTH raw and normalized, got ' + JSON.stringify(sets));
                process.exit(1);
              }
              process.exit(0);
            }).catch((e: unknown) => { console.error(String(e)); process.exit(1); });
            """
        ),
        "redis persist both raw and normalized",
    )
    ok()


def eval_ts_api_web_pagination() -> None:
    run_ts_harness(
        "service/users.ts",
        textwrap.dedent(
            """
            const rows = [];
            for (let i = 0; i < 10; i++) rows.push({ id: String(i), name: 'u'+i });
            const db = { users: { findMany: async (opts: Record<string, unknown> = {}) => {
              let all = rows.slice();
              const take = opts.take ?? opts.limit ?? opts.pageSize ?? opts.page_size;
              let skip = opts.skip ?? opts.offset;
              if (skip == null && opts.page != null) {
                const size = Number(take ?? 10);
                skip = (Number(opts.page) - 1) * size;
              }
              if (typeof skip === 'number') all = all.slice(skip);
              if (typeof take === 'number') all = all.slice(0, take);
              return all;
            }, create: async (d: { data: unknown }) => d.data } };
            function len(got: unknown) {
              if (Array.isArray(got)) return got.length;
              if (got && typeof got === 'object' && Array.isArray((got as { users?: unknown }).users)) {
                return (got as { users: unknown[] }).users.length;
              }
              return -1;
            }
            function idsOf(got: unknown): string {
              const arr = Array.isArray(got) ? got : (got && typeof got === 'object' && Array.isArray((got as { users?: unknown[] }).users) ? (got as { users: unknown[] }).users : []);
              return JSON.stringify(arr.map((r: { id?: unknown }) => r && r.id));
            }
            """
        ),
        textwrap.dedent(
            """
            async function callPage(page: number, limit: number): Promise<{ n: number; ids: string }> {
              const attempts: unknown[] = [];
              try { attempts.push(await listUsers({ page, pageSize: limit, page_size: limit, limit, offset: (page - 1) * limit } as never)); } catch {}
              try { attempts.push(await (listUsers as (a?: unknown, b?: unknown) => unknown)(page, limit)); } catch {}
              const paginated = attempts.find((g) => len(g) === limit);
              if (paginated !== undefined) return { n: limit, ids: idsOf(paginated) };
              for (const got of attempts) {
                const n = len(got);
                if (n >= 0) return { n, ids: idsOf(got) };
              }
              return { n: -1, ids: '' };
            }
            Promise.resolve()
              .then(async () => {
                const a = await callPage(1, 2);
                const b = await callPage(2, 2);
                if (a.n !== 2) {
                  console.error('pagination did not slice to pageSize=2, n=' + a.n);
                  process.exit(1);
                }
                if (b.n < 0) {
                  console.error('second page call failed');
                  process.exit(1);
                }
                if (a.ids === b.ids) {
                  console.error('pagination ignored page/limit; both calls returned ' + a.ids);
                  process.exit(1);
                }
                process.exit(0);
              })
              .catch((e: unknown) => { console.error(String(e)); process.exit(1); });
            """
        ),
        "users list pagination",
    )
    ok()


def eval_ts_api_web_contract_field() -> None:
    run_ts_harness(
        "service/users.ts",
        textwrap.dedent(
            """
            const db = { users: { findMany: async () => [{ id: '1', name: 'Ada' }], create: async (d: { data: unknown }) => d.data } };
            """
        ),
        textwrap.dedent(
            """
            listUsers().then((got: unknown) => {
              const row = Array.isArray(got) ? got[0] : (got && (got as { users?: unknown[] }).users ? (got as { users: unknown[] }).users[0] : got);
              const keys = Object.keys(row || {});
              const renamed = keys.some(k => k !== 'id' && k !== 'name' && /name|display/i.test(k));
              if (keys.includes('name') && !renamed) {
                console.error('name field not renamed: ' + keys);
                process.exit(1);
              }
              if (!renamed) { console.error('no replacement field: ' + keys); process.exit(1); }
              process.exit(0);
            }).catch((e: unknown) => { console.error(String(e)); process.exit(1); });
            """
        ),
        "users contract field rename",
    )
    ok()


def eval_ts_api_web_creation_test() -> None:
    tests = [p for p in list(ROOT.rglob("*.test.ts")) + list(ROOT.rglob("*.spec.ts")) if p.is_file()]
    if not tests:
        fail("no web tests")
    prelude = textwrap.dedent(
        """
        let createUserCalled = false;
        let expectCalled = false;
        async function createUser(..._args: unknown[]) {
          createUserCalled = true;
          return { id: "1", name: "Ada" };
        }
        async function renderUsers() { return ""; }
        const pending: Promise<unknown>[] = [];
        function describe(_n: string, fn: () => void) { fn(); }
        function it(_n: string, fn: () => unknown) {
          pending.push(Promise.resolve().then(() => fn()));
        }
        const test = it;
        function beforeEach(_fn?: unknown) {}
        function afterEach(_fn?: unknown) {}
        const chain: any = new Proxy(function () { return chain; }, {
          get(_t, prop) {
            if (prop === "then") return undefined;
            return chain;
          },
          apply() { return chain; },
        });
        function expect(_v: unknown) {
          expectCalled = true;
          return chain;
        }
        const jest = { fn: () => () => {}, spyOn: () => chain };
        const vi = jest;
        """
    )
    body = textwrap.dedent(
        """
        Promise.all(pending).then(() => {
          if (!createUserCalled) {
            console.error("createUser was not invoked by a test");
            process.exit(1);
          }
          if (!expectCalled) {
            console.error("creation test made no assertion");
            process.exit(1);
          }
          process.exit(0);
        }).catch((e: unknown) => { console.error(String(e)); process.exit(1); });
        """
    )
    for t in tests:
        rel = t.relative_to(ROOT).as_posix()
        try:
            run_ts_harness(rel, prelude, body, "createUser invocation")
            ok()
        except SystemExit as e:
            if e.code in (0, None):
                raise
            continue
    fail("no test file invoked createUser")


def _monorepo_route_bodies() -> tuple[dict, dict]:
    stdout = run_ts_capture(
        "api/routes.ts",
        textwrap.dedent(
            """
            function Router() {
              return {
                get: (_p: string, fn: Function) => { (globalThis as { __get?: Function }).__get = fn; },
                post: (_p: string, fn: Function) => { (globalThis as { __post?: Function }).__post = fn; },
              };
            }
            const prisma = {
              transcript: {
                findUnique: async () => ({
                  id: "t1",
                  raw_text: "hello world",
                  normalized_text: "HELLO",
                  language: "en",
                  createdAt: new Date().toISOString(),
                }),
                create: async () => ({
                  id: "t2",
                  raw_text: "new",
                  normalized_text: "NEW",
                  language: "en",
                  createdAt: new Date().toISOString(),
                }),
              },
            };
            """
        ),
        textwrap.dedent(
            """
            const getFn = (globalThis as { __get?: Function }).__get;
            const postFn = (globalThis as { __post?: Function }).__post;
            if (typeof getFn !== "function") { console.error("GET handler missing"); process.exit(2); }
            if (typeof postFn !== "function") { console.error("POST handler missing"); process.exit(2); }
            const makeRes = (label: string) => ({
              json: (body: unknown) => { console.log(label + JSON.stringify(body)); },
              status: function () { return this; },
            });
            Promise.resolve(getFn({ params: { id: "t1" } }, makeRes("JSONGET:")))
              .then(() => postFn({ body: { text: "new" } }, makeRes("JSONPOST:")))
              .then(() => process.exit(0))
              .catch((e: unknown) => { console.error(String(e)); process.exit(1); });
            """
        ),
        "monorepo GET+POST /api/transcripts",
    )
    get_body = {}
    post_body = {}
    for line in stdout.splitlines():
        if line.startswith("JSONGET:"):
            get_body = json.loads(line[len("JSONGET:"):])
        elif line.startswith("JSONPOST:"):
            post_body = json.loads(line[len("JSONPOST:"):])
    if not get_body:
        fail("GET handler did not print JSON")
    if not post_body:
        fail("POST handler did not print JSON")
    return get_body, post_body


def _assert_transcript_renamed(body_json: dict, label: str) -> None:
    keys = set(body_json.keys())
    has_new = "transcriptText" in keys or "transcript_text" in keys
    if not has_new:
        fail(f"{label} response missing transcriptText: {keys}")
    if "transcript" in keys:
        fail(f"{label} old transcript key still present: {keys}")


def eval_monorepo_rename_transcript_field() -> None:
    get_body, post_body = _monorepo_route_bodies()
    _assert_transcript_renamed(get_body, "GET")
    _assert_transcript_renamed(post_body, "POST")
    ok()


def eval_monorepo_worker_indexing() -> None:
    run_ts_harness(
        "worker/indexer.ts",
        textwrap.dedent(
            """
            const sets: unknown[] = [];
            const redis = {
              lrange: async () => ['{"id":"t1"}'],
              set: async (k: unknown, v: unknown) => sets.push([k, v]),
            };
            const prisma = { transcript: { findUnique: async ({ where }: { where: { id: string } }) => {
              if (where.id === 't1') return { normalized_text: 'NORM' };
              return null;
            }}};
            """
        ),
        textwrap.dedent(
            """
            indexTranscripts().then(() => {
              if (!sets.some((s) => String((s as unknown[])[1]).includes('NORM'))) {
                console.error('did not index JSON event payloads, sets=' + JSON.stringify(sets));
                process.exit(1);
              }
              process.exit(0);
            }).catch((e: unknown) => { console.error(String(e)); process.exit(1); });
            """
        ),
        "worker must index JSON event ids",
    )
    ok()


def eval_monorepo_new_field() -> None:
    get_body, _post_body = _monorepo_route_bodies()
    baseline = {
        "id",
        "transcript",
        "transcriptText",
        "transcript_text",
        "normalizedTranscript",
        "normalized_text",
        "createdAt",
        "created_at",
    }
    extra = set(get_body.keys()) - baseline
    if "language" not in get_body:
        fail(f"response missing added field language: {set(get_body.keys())}")
    if not extra:
        fail(f"response has no extra field beyond a rename: {set(get_body.keys())}")
    ok()


def eval_large_ts_new_order_endpoint() -> None:
    run_ts_harness(
        "src/api/server.ts",
        textwrap.dedent(
            """
            const posts: string[] = [];
            function express() {
              return {
                get: (_p: string, _fn?: unknown) => {},
                post: (p: string, _fn?: unknown) => { posts.push(p); },
                listen: () => {},
              };
            }
            async function listOrders() { return []; }
            async function getOrder(_id: string) { return {}; }
            const web = { renderHome: async () => "" };
            """
        ),
        textwrap.dedent(
            """
            if (!posts.includes("/api/orders")) {
              console.error("POST /api/orders not registered: " + JSON.stringify(posts));
              process.exit(1);
            }
            process.exit(0);
            """
        ),
        "POST /api/orders must be registered",
    )
    ok()


def eval_large_ts_order_total_invariant() -> None:
    run_ts_harness(
        "src/domain/orders.ts",
        "",
        textwrap.dedent(
            """
            const inserted: unknown[] = [];
            const repo = { insert: async (row: unknown) => { inserted.push(row); return row; }, list: async () => [], get: async () => ({}) };
            const svc = new OrderService(repo);
            (svc as { repo?: unknown }).repo = repo;
            Promise.resolve(svc.createOrder([{ price: -1, qty: 1 }])).then((row: unknown) => {
              console.error('negative price must be rejected, got ' + JSON.stringify(row));
              process.exit(1);
            }).catch(() => process.exit(0));
            """
        ),
        "order total invariant must reject invalid lines",
    )
    ok()


def eval_nextjs_transcript_response() -> None:
    run_ts_harness(
        "app/api/transcripts/route.ts",
        textwrap.dedent(
            """
            class TranscriptStore {
              async find(id: string) { return { id, raw_text: "hello", createdAt: new Date().toISOString() }; }
              async list() { return [{ id: "t1", raw_text: "hello" }]; }
              async save(text: string) { return { id: "t2", raw_text: text }; }
            }
            function keysOk(body: unknown, label: string) {
              const keys = Object.keys((body && typeof body === "object") ? body as object : {});
              const has = keys.includes("transcriptText") || keys.includes("transcript_text");
              if (!has) { console.error(label + " missing transcriptText: " + keys); process.exit(1); }
              if (keys.includes("transcript")) {
                console.error(label + " still wraps transcript: " + keys);
                process.exit(1);
              }
            }
            """
        ),
        textwrap.dedent(
            """
            if (typeof GET !== "function" || typeof POST !== "function") {
              console.error("GET/POST missing");
              process.exit(1);
            }
            const getReq = new Request("http://localhost/api/transcripts?id=t1");
            const postReq = new Request("http://localhost/api/transcripts", {
              method: "POST",
              headers: { "content-type": "application/json" },
              body: JSON.stringify({ text: "new" }),
            });
            Promise.all([GET(getReq as never), POST(postReq as never)]).then(async (resps) => {
              const getBody = await resps[0].json();
              const postBody = await resps[1].json();
              keysOk(getBody, "GET");
              keysOk(postBody, "POST");
              process.exit(0);
            }).catch((e: unknown) => { console.error(String(e)); process.exit(1); });
            """
        ),
        "nextjs transcript response JSON",
    )
    ok()


def eval_nextjs_store_pagination() -> None:
    run_ts_harness(
        "lib/transcripts.ts",
        textwrap.dedent(
            """
            let lastSql = "";
            function sql(strings: TemplateStringsArray, ...vals: unknown[]) {
              lastSql = [...strings].join("?") + " " + vals.map(String).join(",");
              return Promise.resolve([{ id: "a" }, { id: "b" }, { id: "c" }]);
            }
            """
        ),
        textwrap.dedent(
            """
            const store = new TranscriptStore();
            const tryList = async () => {
              let a: unknown = null;
              let b: unknown = null;
              try { a = await store.list({ limit: 1, offset: 0, page: 1, pageSize: 1 } as never); } catch {}
              const sqlA = lastSql;
              try { b = await store.list({ limit: 2, offset: 2, page: 2, pageSize: 2 } as never); } catch {}
              const sqlB = lastSql;
              if (Array.isArray(a) && Array.isArray(b) && a.length !== b.length) { process.exit(0); }
              if (sqlA && sqlB && sqlA !== sqlB && /limit|offset|page/i.test(sqlA + " " + sqlB)) { process.exit(0); }
              console.error("list is not paginated sql=" + lastSql + " a=" + JSON.stringify(a) + " b=" + JSON.stringify(b));
              process.exit(1);
            };
            tryList().catch((e: unknown) => { console.error(String(e)); process.exit(1); });
            """
        ),
        "TranscriptStore.list pagination",
    )
    ok()


def eval_polyglot_refund_endpoint() -> None:
    import io
    import types
    svc = ROOT / "svc"
    if not (svc / "payments.py").is_file() or not (svc / "server.py").is_file():
        fail("payments.py or server.py missing")
    prev_path = list(sys.path)
    prev_db = sys.modules.get("db")
    db_mod = types.ModuleType("db")
    class PaymentDb:
        def insert(self, amount):
            return {"id": "p1", "amount": amount}
        def recent(self):
            return []
    db_mod.PaymentDb = PaymentDb
    sys.modules["db"] = db_mod
    sys.path.insert(0, str(svc))
    try:
        pay = load_py("svc/payments.py", "payments_eval")
        fn = getattr(pay, "handle_refund", None) or getattr(pay, "refund", None)
        if not callable(fn):
            fail("handle_refund missing")
        calls: list = []
        def wrapped(*a, **k):
            calls.append((a, k))
            try:
                return fn(*a, **k)
            except TypeError:
                return fn()
        pay.handle_refund = wrapped
        if hasattr(pay, "refund"):
            pay.refund = wrapped
        sys.modules["payments"] = pay
        server = load_py("svc/server.py", "server_eval")
        for name in ("handle_refund", "refund"):
            if hasattr(server, name):
                setattr(server, name, wrapped)
        handler_cls = getattr(server, "Handler", None)
        if handler_cls is None:
            fail("Handler missing")
        paths = ("/refund", "/payments/refund", "/api/refund", "/api/payments/refund")
        methods = [n for n in dir(handler_cls) if n.startswith("do_")]
        for method_name in methods:
            for path in paths:
                calls.clear()
                handler = object.__new__(handler_cls)
                handler.path = path
                handler.command = method_name[3:]
                handler.headers = {"Content-Length": "0"}
                handler.rfile = io.BytesIO(b"")
                handler.wfile = io.BytesIO()
                handler.send_response = lambda *a, **k: None
                handler.send_header = lambda *a, **k: None
                handler.end_headers = lambda *a, **k: None
                handler.log_message = lambda *a, **k: None
                try:
                    getattr(handler, method_name)()
                except Exception:
                    pass
                if calls:
                    return ok()
        fail("refund not dispatched through HTTP handler")
    finally:
        sys.path[:] = prev_path
        if str(svc) in sys.path:
            try:
                sys.path.remove(str(svc))
            except ValueError:
                pass
        if prev_db is None:
            sys.modules.pop("db", None)
        else:
            sys.modules["db"] = prev_db


def eval_polyglot_web_payments() -> None:
    run_ts_harness(
        "web/app.ts",
        textwrap.dedent(
            """
            const api = {
              get: async () => ({ data: [{ id: "p1", amount: 199, currency: "USD" }] }),
            };
            """
        ),
        textwrap.dedent(
            """
            Promise.resolve(renderPayments()).then((html) => {
              const s = String(html);
              const hasAmount = /199|1\\.99/.test(s);
              const hasCurrency = /USD|EUR|GBP|currency|cents/i.test(s);
              if (!hasAmount || !hasCurrency) {
                console.error("web must render amount 199 and a currency, got: " + s);
                process.exit(1);
              }
              process.exit(0);
            }).catch((e: unknown) => { console.error(String(e)); process.exit(1); });
            """
        ),
        "web payment currency rendering",
    )
    ok()


# ---------------------------------------------------------------------------
# Dispatch — one behavioral evaluator per canonical tasks.json id.
# String/grep detectors are forbidden: a comment-only patch MUST fail.
# ---------------------------------------------------------------------------
DISPATCH = {
    "http-service.rename-transcript-field": eval_http_service_rename_transcript_field,
    "http-service.health-check": eval_http_service_health_check,
    "http-service.transcript-normalization": eval_http_service_transcript_normalization,
    "py-queue.empty-messages": eval_py_queue_empty_messages,
    "py-queue.classification-fallback": eval_py_queue_classification_fallback,
    "py-queue.store-changes": eval_py_queue_store_changes,
    "queue-worker.asr-retry": eval_queue_worker_asr_retry,
    "queue-worker.street-vocabulary": eval_queue_worker_street_vocabulary,
    "queue-worker.redis-persist": eval_queue_worker_redis_persist,
    "ts-api-web.pagination": eval_ts_api_web_pagination,
    "ts-api-web.contract-field": eval_ts_api_web_contract_field,
    "ts-api-web.creation-test": eval_ts_api_web_creation_test,
    "monorepo.rename-transcript-field": eval_monorepo_rename_transcript_field,
    "monorepo.worker-indexing": eval_monorepo_worker_indexing,
    "monorepo.new-field": eval_monorepo_new_field,
    "large-ts.new-order-endpoint": eval_large_ts_new_order_endpoint,
    "large-ts.order-total-invariant": eval_large_ts_order_total_invariant,
    "nextjs.transcript-response": eval_nextjs_transcript_response,
    "nextjs.store-pagination": eval_nextjs_store_pagination,
    "polyglot.refund-endpoint": eval_polyglot_refund_endpoint,
    "polyglot.web-payments": eval_polyglot_web_payments,
}


SCOPED_SIX = [
    "http-service.rename-transcript-field",
    "queue-worker.asr-retry",
    "ts-api-web.pagination",
    "py-queue.empty-messages",
    "http-service.health-check",
    "queue-worker.street-vocabulary",
]


# trace:v1 id=impl.scc.evaluators.evaluate-task work=WORK-p0-omp-integration-correctness-and-writable-benchmark-scientific-validit satisfies=REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient
def evaluate_task(task_id: str, repo: Path) -> tuple[bool, str]:
    """Run one evaluator against `repo`. Used by tests; the harness invokes
    this file as a subprocess with cwd = the edited copy."""
    import contextlib
    import io

    global ROOT
    fn = DISPATCH.get(task_id)
    if fn is None:
        return False, f"unknown task id {task_id}"
    prev = ROOT
    prev_path = list(sys.path)
    prev_db = sys.modules.get("db")
    ROOT = Path(repo)
    buf = io.StringIO()
    try:
        with contextlib.redirect_stderr(buf), contextlib.redirect_stdout(buf):
            fn()
        return True, "ok"
    except SystemExit as e:
        detail = buf.getvalue().strip()
        if e.code in (0, None):
            return True, detail or "ok"
        return False, detail or f"exit {e.code}"
    except Exception as e:
        return False, f"{type(e).__name__}: {e}\n{buf.getvalue()}"
    finally:
        ROOT = prev
        sys.path[:] = prev_path
        if prev_db is None:
            sys.modules.pop("db", None)
        else:
            sys.modules["db"] = prev_db


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        sys.stderr.write("usage: run.py <task-id>\n")
        return 2
    task_id = argv[1]
    fn = DISPATCH.get(task_id)
    if fn is None:
        fail(f"unknown task id {task_id}")
    try:
        fn()
    except SystemExit:
        raise
    except Exception as e:
        fail(f"evaluator crashed: {type(e).__name__}: {e}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
