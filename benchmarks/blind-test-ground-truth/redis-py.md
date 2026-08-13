# redis-py
> https://github.com/redis/redis-py | Python | python redis client | ~70k LOC

## architecture
- redis — the package root: Redis, AsyncRedis, connection, commands (redis/)
- client.py — the sync client: Redis, Pipeline, PubSub, Monitor (redis/client.py)
- asyncio/client.py — the async client: Redis, Pipeline, PubSub (redis/asyncio/client.py)
- connection.py — connection management: Connection, SSLConnection, ConnectionPool, UnixDomainSocketConnection (redis/connection.py)
- commands — the command namespaces: CoreCommands, RedisModuleCommands, SentinelCommands (redis/commands/)
- cluster.py — cluster client: RedisCluster (redis/cluster.py)
- sentinel.py — sentinel client: Sentinel, SentinelManagedConnection (redis/sentinel.py)
- _parsers — response parsing: RESP2Parser, PythonParser (redis/_parsers/)
- lock.py — the lock implementation: Lock (redis/lock.py)
- retry.py — retry policy: Retry (redis/retry.py)
- cache.py — client-side caching: Cache, CacheInterface (redis/cache.py)
- auth — authentication providers (redis/auth/)
- backoff.py — backoff strategies: ExponentialWithJitterBackoff, NoBackoff (redis/backoff.py)
- data_structure.py — advanced data structures (redis/data_structure.py)
- asyncio — the async mirror of the sync modules (redis/asyncio/)

## entrypoints
- redis.Redis — the sync client entry
- redis.asyncio.Redis — the async client entry
- redis.Redis.from_url — client from URL
- redis.RedisCluster — the cluster client entry
- redis.Sentinel — the sentinel client entry
- redis.ConnectionPool — the pool entry
- redis.StrictRedis — legacy strict client alias
- redis.Pipeline — the pipeline entry
- redis.PubSub — the pub/sub entry
- redis.Lock — the lock entry
- redis.lock.Lock — the distributed lock
- redis.Connection — the raw connection entry
- redis.Retry — the retry entry
- redis.backoff — backoff strategy entry
- redis.cluster.RedisCluster.from_url — cluster from URL

## behavior
- Redis.execute_command -> connection -> RESP response — command execution (client.py)
- Redis.pipeline() -> execute -> commands batch — pipeline flow (client.py)
- PubSub.subscribe -> connection listen loop — pub/sub flow (client.py)
- ConnectionPool.get_connection -> acquire -> release — pool lifecycle (connection.py)
- Lock.acquire -> set nx -> lock — lock acquisition (lock.py)
- Retry.call_with_retry -> backoff -> retry — retry flow (retry.py)
- Sentinel.discover_master -> get_master_addr_by_name — master discovery (sentinel.py)
- RedisCluster.execute_command -> node routing — cluster routing (cluster.py)

## state_authority
- Redis — the client state: connection pool, response callbacks (client.py)
- ConnectionPool — the connection pool state (connection.py)
- Connection — per-connection state: socket, parser, options (connection.py)
- PubSub — subscription state (client.py)
- Lock — the lock state: token, timeout, owner (lock.py)
- Pipeline — the batched command buffer (client.py)
- RedisCluster — the cluster topology state (cluster.py)
- Sentinel — the sentinel state (sentinel.py)
- Cache — the client-side cache state (cache.py)

## contracts
- redis:// — connection URL scheme
- rediss:// — TLS connection URL scheme
- unix:// — unix socket URL scheme
- GET key — get command contract
- SET key value — set command contract
- DEL key — delete command contract
- EXPIRE key seconds — expiry contract
- HSET hash field value — hash set contract
- LPUSH list value — list push contract
- PUBLISH channel message — publish contract
- SUBSCRIBE channel — subscribe contract
- INCR key — increment contract
- SELECT db — database select contract
- AUTH password — auth contract
- PING — ping contract
- MGET key1 key2 — multi-get contract

## landmarks
- Redis — the sync client (client.py)
- Redis — the async client (asyncio/client.py)
- ConnectionPool — the pool (connection.py)
- Connection — the connection (connection.py)
- PubSub — pub/sub (client.py)
- Pipeline — pipelining (client.py)
- RedisCluster — cluster client (cluster.py)
- Sentinel — sentinel client (sentinel.py)
- Lock — the lock (lock.py)
- Retry — retry policy (retry.py)
- Cache — client-side cache (cache.py)
- RESP2Parser — the RESP2 parser (_parsers/)

## tests
- tests/test_client.py — client tests
- tests/test_pubsub.py — pub/sub tests
- tests/test_pipeline.py — pipeline tests
- tests/test_connection_pool.py — pool tests
- tests/test_cluster.py — cluster tests
- tests/test_sentinel.py — sentinel tests
- tests/test_lock.py — lock tests
- tests/test_asyncio/test_client.py — async client tests
- tests/test_commands.py — command tests
