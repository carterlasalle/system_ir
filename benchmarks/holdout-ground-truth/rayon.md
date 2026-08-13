# rayon
> https://github.com/rayon-rs/rayon | Rust | rust lib | ~44k LOC

## architecture
- src/iter — parallel iterator implementations: ParallelIterator trait + adapters (src/iter/)
- src/slice — parallel slice operations: par_sort, par_chunks (src/slice/)
- src/vec — parallel vec methods (src/vec.rs)
- src/collections — parallel collection extension traits (src/collections/)
- rayon-core — the thread pool runtime crate (rayon-core/src/)
- src/join — the join primitive (src/join.rs)
- src/scope — the scope primitive (src/scope.rs)
- src/thread_pool — thread pool management (src/thread_pool.rs)
- src/prelude — the trait bundle users import (src/prelude.rs)

## entrypoints
- par_iter — parallel iterator entry on slices/vecs
- par_iter_mut — mutable parallel iterator entry
- par_sort — parallel slice sort (src/slice/sort.rs)
- join — split-and-parallel function call (src/join.rs)
- scope — scoped parallel task creation (src/scope.rs)
- ThreadPoolBuilder — thread pool construction (src/thread_pool.rs)
- par_chunks — parallel chunk iteration
- par_extend — parallel collection growth
- into_par_iter — owning parallel iterator
- current_num_threads — pool introspection entry

## behavior
- join(a, b) -> task spawn -> work-stealing execution — parallel fork/join (src/join.rs)
- par_iter -> ParallelIterator::map -> find_any/for_each — adapter chain execution (src/iter/plumbing/)
- par_sort -> merge sort with parallel merges — parallel sorting (src/slice/sort.rs)
- scope(|s| s.spawn(...)) -> scope registration -> execute tasks — scoped tasks (src/scope.rs)
- ThreadPoolBuilder::build -> new thread pool + Registry — pool construction (src/thread_pool.rs)
- Registry::new -> spawn worker threads -> steal loop — worker startup (rayon-core/src/registry.rs)
- par_iter -> split -> drive_unindexed -> consume each item — splitting iteration (src/iter/plumbing/)
- bridge consumer: fold/reduce across splits — reduction

## state_authority
- Registry — the global thread-pool registry (rayon-core/src/registry.rs)
- ThreadPool — the pool handle (src/thread_pool.rs)
- WorkerThread — per-thread worker state (rayon-core/src/worker.rs)
- Scope — scope task state (src/scope.rs)
- global registry — the default global pool (rayon-core/src/registry.rs)
- sleep — thread park/sleep state (rayon-core/src/sleep.rs)
- job registry — pending job bookkeeping

## contracts
- use rayon::prelude::* — trait import contract (src/prelude.rs)
- v.par_iter().map(f).collect() — parallel iterator contract
- v.par_iter_mut().for_each(f) — mutable parallel iteration
- v.par_sort() — parallel sort contract
- join(f, g) — fork/join contract
- scope(|s| { s.spawn(move || ...); }) — scope spawn contract
- ThreadPoolBuilder::new().num_threads(n).build() — pool config contract
- v.par_chunks(n) — chunked iteration contract
- par_extend into existing collection — growth contract
- par_iter().filter().fold() — adapter chaining contract

## landmarks
- ParallelIterator — the core trait (src/iter/mod.rs)
- IndexedParallelIterator — indexed traversal trait (src/iter/mod.rs)
- ThreadPoolBuilder — pool builder (src/thread_pool.rs)
- Registry — runtime registry (rayon-core/src/registry.rs)
- WorkerThread — worker state (rayon-core/src/worker.rs)
- sleep — sleep state machine (rayon-core/src/sleep.rs)
- Splitter — splitting heuristics (src/iter/splitter.rs)
- par_bridge — iterator bridging (src/iter/from_par_iter.rs)

## tests
- src/tests — inline module tests
- tests/ — integration tests (tests/compile_fail, tests/run-pass)
- rayon-core/tests/ — runtime tests
- tests/run-pass/ — passing behavior tests
- tests/compile_fail/ — compile-time error checks
