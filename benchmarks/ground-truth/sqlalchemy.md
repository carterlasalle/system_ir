# sqlalchemy
> https://github.com/sqlalchemy/sqlalchemy | Python | db-heavy | ~632k LOC

## components
- Engine — DB-API wrapper/connection factory in lib/sqlalchemy/engine/base.py (line 2893)
- Connection — per-call DBAPI connection wrapper with transactions (lib/sqlalchemy/engine/base.py line 90)
- Session — ORM persistence unit-of-work class in lib/sqlalchemy/orm/session.py (line 1471)
- sessionmaker — configurable Session factory (lib/sqlalchemy/orm/session.py line 5100)
- scoped_session — thread-local Session scoping (lib/sqlalchemy/orm/scoping.py line 153)
- Mapper — class-to-table mapping definition (lib/sqlalchemy/orm/mapper.py line 168)
- Select — 2.0-style selectable statement (lib/sqlalchemy/sql/selectable.py line 5449)
- Table — schema table construct (lib/sqlalchemy/sql/schema.py line 328)
- Column — schema column construct (lib/sqlalchemy/sql/schema.py line 1770)
- MetaData — collection of Table objects with DDL (lib/sqlalchemy/sql/schema.py line 5788)
- ForeignKey — column-level referential constraint (lib/sqlalchemy/sql/schema.py line 3036)
- Pool — abstract connection pool (lib/sqlalchemy/pool/base.py line 156); QueuePool default in pool/impl.py line 44
- SQLiteDialect — sqlite dialect class with name = "sqlite" (lib/sqlalchemy/dialects/sqlite/base.py line 2102)
- PGDialect — postgresql dialect class with name = "postgresql" (lib/sqlalchemy/dialects/postgresql/base.py line 3590)
- MySQLDialect — mysql dialect class (lib/sqlalchemy/dialects/mysql/base.py line 2723)

## entrypoints
- create_engine — builds an Engine from a URL string (lib/sqlalchemy/engine/create.py line 117)
- declarative_base — factory for declarative mapping base classes (lib/sqlalchemy/orm/decl_api.py line 1042)
- select — 2.0 SELECT constructor (lib/sqlalchemy/sql/_selectable_constructors.py line 574)
- Engine.connect — returns a new Connection (lib/sqlalchemy/engine/base.py line 3237)
- Connection.execute — executes a statement against the DBAPI (lib/sqlalchemy/engine/base.py line 1404)
- Session.execute — ORM-aware statement execution (lib/sqlalchemy/orm/session.py line 2387)
- Session.connection — binds the session to a Connection (lib/sqlalchemy/orm/session.py line 2095)
- MetaData.create_all — emits CREATE TABLE DDL for all tables (lib/sqlalchemy/sql/schema.py line 6295)
- registry — declarative class registry (lib/sqlalchemy/orm/decl_api.py line 1165)

## flows
- create_engine -> Engine.connect -> Connection.execute — core execution path (engine/base.py)
- Session.execute -> ORMExecuteState — hook state passed to do_orm_execute events (orm/session.py line 257)
- declarative_base -> Mapper.configure — declarative class config into mapper (orm/decl_base.py _DeclarativeMapperConfig)
- Table -> MetaData.create_all — DDL generation from schema constructs (sql/schema.py line 6295)
- Transaction -> commit/rollback — SessionTransaction lifecycle (orm/session.py line 858)
- QueuePool -> _ConnectionFairy — connection checkout/return proxy (pool/base.py line 1193)

## ownership
- engine.pool — the Pool instance owning DBAPI connections (engine/base.py Engine.pool)
- Session identity map — IdentityMap keyed by instance identity (lib/sqlalchemy/orm/identity.py line 37)
- MetaData.tables — dict of Table objects owned by the MetaData (sql/schema.py)
- sessionmaker(bind=engine) — factory binding sessions to an Engine (orm/session.py line 5100)
- _sa_instance_state — InstanceState tracking per-instance mapper state (lib/sqlalchemy/orm/state.py line 105)
- Session.connection — session-level transaction ownership

## contracts
- create_engine("sqlite://") — in-memory sqlite URL contract in test/aaa_profiling/test_resultset.py (line 193)
- "sqlite:///:memory:" — memory DB URL flagged in test/aaa_profiling/test_memusage.py (line 507)
- Table("sometable", m, Column("somecolumn", String)) — table/column construction in test/dialect/mssql/test_compiler.py (line 113)
- LABEL_STYLE_TABLENAME_PLUS_COL — label-style contract in test/dialect/mssql/test_deprecations.py (line 107)
- select(t1).where(t1.c.c2 == t2.c.c1) — select/where usage in test/aaa_profiling/test_compiler.py (line 73)
- test_session_commit_rollback — commit/rollback lifecycle test (test/aaa_profiling/test_memusage.py line 1691)
- name = "sqlite" — dialect URL scheme identifier (lib/sqlalchemy/dialects/sqlite/base.py line 2103)

## tests
- test/orm/test_session.py — Session behavior tests
- test/orm/test_query.py — ORM Query tests
- test/engine/test_transaction.py — transaction lifecycle tests
- test/engine/test_pool.py — pool behavior tests
- test/dialect/sqlite/ — sqlite dialect compiler/reflection/types tests
- test/aaa_profiling/test_orm.py — SessionTest/QueryTest memory profiling
- test/aaa_profiling/test_pool.py — QueuePoolTest memory profiling
