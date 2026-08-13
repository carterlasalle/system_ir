# guava
> https://github.com/google/guava | Java | java lib | ~1045k LOC

## architecture
- com.google.common.collect — collection utilities: immutable collections, multimaps, biMap (guava/src/com/google/common/collect/)
- com.google.common.base — foundational utilities: Preconditions, Strings, Optional (base/)
- com.google.common.cache — the Cache API: CacheBuilder, CacheLoader, LoadingCache (cache/)
- com.google.common.hash — hashing primitives: Hashing, HashFunction (hash/)
- com.google.common.io — I/O utilities: Files, CharStreams (io/)
- com.google.common.net — networking helpers: InternetDomainName, MediaType (net/)
- com.google.common.primitives — primitive helpers: Ints, Longs (primitives/)
- com.google.common.reflect — reflection helpers: TypeToken, Invokable (reflect/)
- com.google.common.eventbus — the EventBus (eventbus/)
- com.google.common.graph — graph APIs: Graph, ValueGraph (graph/)

## entrypoints
- ImmutableList.of — immutable list factory
- ImmutableMap.of — immutable map factory
- Lists.newArrayList — mutable list factory
- Maps.newHashMap — mutable map factory
- Sets.newHashSet — mutable set factory
- CacheBuilder.newBuilder — cache construction entry (cache/CacheBuilder.java)
- Preconditions.checkNotNull — null-check entry (base/Preconditions.java)
- Optional.of — optional value entry
- MoreObjects.toStringHelper — string building entry
- Hashing.md5 — hash function entry (hash/Hashing.java)
- CharStreams.toString — stream reading entry (io/CharStreams.java)

## behavior
- CacheBuilder.build -> LocalCache creation -> CacheLoader — cache construction (cache/CacheBuilder.java)
- LocalCache.get -> segment lookup -> load via CacheLoader — cache load flow (cache/LocalCache.java)
- ImmutableList.copyOf -> copyIntoRegularList — immutable copy (collect/ImmutableList.java)
- ImmutableCollection.add -> UnsupportedOperationException — immutability enforcement
- RegularImmutableMap.get -> hash lookup — map access
- EventBus.post -> subscriber dispatch — event dispatch (eventbus/EventBus.java)
- Hashing.md5 -> hashFunction.newHasher -> hash — hashing pipeline
- Preconditions.checkArgument -> IllegalArgumentException — precondition failure flow

## state_authority
- Cache — the cache interface (cache/Cache.java)
- CacheBuilder — cache configuration state (cache/CacheBuilder.java)
- LocalCache — the segment-based cache implementation (cache/LocalCache.java)
- ImmutableCollection — immutable collection state
- EventBus — subscriber registry (eventbus/EventBus.java)
- Interners — intern pool state (collect/Interners.java)
- TypeToken — runtime type state (reflect/TypeToken.java)
- StatsCounter — cache statistics (cache/AbstractCache.java)

## contracts
- ImmutableList.of(a, b, c) — immutable list factory contract
- ImmutableMap.of(k1, v1, k2, v2) — immutable map factory contract
- CacheBuilder.newBuilder().maximumSize(n).build(loader) — cache build contract
- cache.get(key, callable) — cache load contract
- cache.put(key, value) — cache store contract
- Preconditions.checkArgument(cond, msg) — precondition contract
- Preconditions.checkState(cond) — state check contract
- Optional.of(x).isPresent() — optional contract
- MoreObjects.firstNonNull(a, b) — first-non-null contract
- Hashing.sha256() — hash algorithm contract
- ImmutableMultimap.of(k, v1, v2) — multimap contract
- TypeToken.of(MyType.class) — type token contract

## landmarks
- ImmutableList — immutable list (collect/ImmutableList.java)
- ImmutableMap — immutable map (collect/ImmutableMap.java)
- Multimap — multimap interface (collect/Multimap.java)
- CacheBuilder — cache builder (cache/CacheBuilder.java)
- LoadingCache — cache with loader (cache/LoadingCache.java)
- Preconditions — precondition checks (base/Preconditions.java)
- Optional — optional value (base/Optional.java)
- MoreObjects — object helpers (base/MoreObjects.java)
- EventBus — event bus (eventbus/EventBus.java)
- TypeToken — generic type capture (reflect/TypeToken.java)
- FluentIterable — fluent iteration (collect/FluentIterable.java)

## tests
- guava-tests/ — the main guava test tree
- guava-tests/test/com/google/common/collect/ — collect tests
- guava-tests/test/com/google/common/cache/ — cache tests
- guava-tests/test/com/google/common/base/ — base tests
- guava-tests/test/com/google/common/hash/ — hash tests
- guava-tests/test/com/google/common/eventbus/ — eventbus tests
- guava-testlib/ — test helpers for collection contracts
