# celery
> https://github.com/celery/celery | Python | queue+events | ~102k LOC

## architecture
- Celery — the application class in celery/app/base.py (line 252), central registry for tasks/config
- Task — task base class in celery/app/task.py (line 206), executed by workers
- Signature — task signature (args/kwargs/options) in celery/canvas.py (line 234)
- group — parallel task group primitive (celery/canvas.py line 1493)
- chain — sequential task chain primitive (celery/canvas.py line 1323)
- chord — barrier primitive with header + body (celery/canvas.py _chord, line 1972)
- AsyncResult — task result handle in celery/result.py (line 70)
- GroupResult — group result handle (celery/result.py line 930)
- WorkController — unmanaged worker instance (celery/worker/worker.py line 63)
- RedisBackend — redis result backend (celery/backends/redis.py, __all__ line 38)
- RPCBackend — amqp-based result backend (celery/backends/rpc.py line 154)
- BaseKeyValueStoreBackend — key-value store backend base (celery/backends/base.py line 965)
- Scheduler — periodic task scheduler (celery/beat.py line 219)

## entrypoints
- @app.task — decorator creating a task from a callable (celery/app/base.py line 515); e.g. examples/next-steps/proj/tasks.py line 4
- @shared_task — decorator for reusable library tasks (celery/app/__init__.py line 24); e.g. examples/django/demoapp/tasks.py line 8
- app.send_task — publishes a task message by name (celery/app/base.py line 846)
- task.delay — star-arg shortcut for apply_async (celery/app/task.py line 534)
- task.apply_async — sends the task as a broker message (celery/app/task.py line 547)
- app.worker_main — runs the celery worker programmatically (celery/app/base.py line 499)
- app.start — runs the celery CLI with argv (celery/app/base.py line 480)
- celery -A proj worker — CLI invocation documented in examples/app/myapp.py line 16

## behavior
- apply_async — task dispatch path first step (`apply_async -> send_task -> broker publish`, celery/app/task.py -> base.py)
- task.retry — re-queues the task with countdown/eta (celery/app/task.py line 767)
- chord header -> add_unlock_chord_task — fallback chord join via 'celery.chord_unlock' builtin (celery/app/builtins.py line 37)
- group — group execution first step (`group -> GroupResult`, celery/result.py line 930)
- Scheduler — beat scheduling path first step (`Scheduler -> ScheduleEntry -> apply_async`, celery/beat.py line 82)
- worker consumer -> task.run — worker executes the Task body (celery/app/task.py line 527)

## state_authority
- app.conf — pending config dict (PendingConfiguration, celery/app/base.py line 208)
- broker_url — broker connection setting in examples/eventlet/celeryconfig.py (line 10)
- result_backend — result store setting in examples/gevent/celeryconfig.py (line 10)
- task_keyprefix = 'celery-task-meta-' — redis key prefix owned by the backend (celery/backends/base.py line 967)
- celery/worker/state.py — worker state persisted between restarts (celery/worker/state.py line 192)
- PersistentScheduler — shelve-backed beat scheduler (celery/beat.py line 505)

## contracts
- --broker — CLI broker option with BROKER_URL envvar (celery/bin/celery.py line 67)
- --result-backend — CLI result backend option with RESULT_BACKEND envvar (celery/bin/celery.py line 72)
- -Q — worker queue selection option (celery/bin/worker.py line 254)
- --concurrency — worker concurrency option (celery/bin/worker.py line 195)
- --pool — pool implementation option (prefork default, celery/bin/worker.py line 206)
- broker='amqp://guest@localhost//' — amqp broker URL contract in examples/app/myapp.py (line 31)
- redis://localhost:6379/0 — redis broker/backend URL in examples/security/mysecureapp.py (line 32)
- name='celery.accumulate' — builtin task name in celery/app/builtins.py (line 29)

## landmarks
- Context — Task.request variable container (celery/app/task.py line 75)
- crontab — cron-style schedule in celery/schedules.py (line 331)

## tests
- t/unit/ — unit test suite (app, backends, bin, canvas, worker subdirs)
- t/integration/ — integration tests against live brokers/backends (test_canvas.py, test_backend.py, test_worker.py)
- t/unit/app/test_app.py — Celery app behavior tests
- t/unit/tasks/test_canvas.py — group/chain/chord unit tests
- t/unit/backends/ — result backend unit tests
- t/unit/worker/ — worker component tests
