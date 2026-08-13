# django
> https://github.com/django/django | Python | python service (framework) | ~1144k LOC

## architecture
- django.conf.settings — LazySettings settings object loaded from DJANGO_SETTINGS_MODULE (django/conf/__init__.py)
- django.apps — application registry (apps.populate, AppConfig) loading INSTALLED_APPS
- django.urls — URLconf system: URLResolver, URLPattern, path/re_path/include (django/urls/)
- django.db.models — ORM: Model, QuerySet, ModelManager, fields
- django.core.management — command framework: BaseCommand, CommandParser, command discovery (find_commands/get_commands)
- django.http — request/response cycle: HttpRequest, HttpResponse
- django.middleware — middleware framework (django/middleware/)
- django.core.exceptions — exception hierarchy (ImproperlyConfigured, SuspiciousOperation)

## entrypoints
- django-admin — the core management-command runner (django/core/management/__init__.py execute_from_command_line)
- manage.py — per-project command wrapper delegating to django-admin
- call_command — programmatic management-command invocation (django/core/management/__init__.py)
- django.setup — one-time bootstrap: configure logging, populate app registry (django/__init__.py)
- path — URL pattern function in django/urls/conf.py
- re_path — regex URL pattern function in django/urls/conf.py
- include — URLconf inclusion function in django/urls/conf.py
- reverse — URL reverse resolution (django/urls/base.py)
- resolve — URL path resolution to a view (django/urls/base.py)
- get_commands — command-name discovery registry (django/core/management/__init__.py)

## behavior
- django.setup -> apps.populate(INSTALLED_APPS) — app registry bootstrap (django/__init__.py)
- get_resolver -> URLResolver.resolve -> ResolverMatch — URL dispatch (django/urls/resolvers.py)
- execute_from_command_line -> get_commands -> load_command_class -> BaseCommand.execute — management command dispatch
- URLResolver.resolve -> match -> view callback invocation — request routing
- settings.lazy load: first settings access imports settings module and checks globals — settings initialization
- BaseCommand.execute -> handle — command body execution (django/core/management/base.py)

## state_authority
- django.conf.settings — global settings singleton (LazySettings)
- django.apps.apps — global application registry instance
- get_urlconf — current thread's URLconf module (django/urls/base.py)
- CommandParser — per-command argument parser (django/core/management/base.py)
- connection — database connection (django/db/__init__.py, connections registry)
- INSTALLED_APPS — the app list that drives apps.populate (django/conf/global_settings.py)

## contracts
- python manage.py <command> — management command invocation contract
- DJANGO_SETTINGS_MODULE — environment variable selecting the settings module
- --settings — django-admin flag overriding the settings module
- path('route/', views.index) — route+view contract (django/urls/conf.py)
- re_path(r'^articles/(?P<year>[0-9]{4})/$', views.year_archive) — regex route contract
- include('app.urls') — URLconf inclusion contract
- command <app_label>.<Command> — command name to app mapping (get_commands)
- call_command('migrate', verbosity=0) — programmatic command invocation

## landmarks
- BaseCommand — base class for management commands (django/core/management/base.py)
- CommandParser — argparse subclass used by commands
- AppConfig — app configuration class (django/apps/config.py)
- Model — ORM base model class (django/db/models/base.py)
- QuerySet — lazy database query set (django/db/models/query.py)
- URLResolver — top-level URL router (django/urls/resolvers.py)
- URLPattern — single URL pattern (django/urls/resolvers.py)
- ResolverMatch — result of URL resolution
- MiddlewareMixin — middleware base class (django/utils/deprecation.py)

## tests
- tests/ — the Django test suite, one directory per feature (tests/model_fields/, tests/urls/, etc.)
- tests/urls/ — URLconf behavior tests
- tests/model_fields/ — ORM field tests
- tests/known_good_python/ — supported-interpreter matrix
