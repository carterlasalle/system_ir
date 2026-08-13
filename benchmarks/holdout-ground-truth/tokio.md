# tokio
> https://github.com/tokio-rs/tokio | Rust | rust lib (runtime) | ~191k LOC

## architecture
- tokio/src/runtime — the async runtime: scheduler, driver, blocking pool (tokio/src/runtime/)
- tokio/src/task — task spawning and JoinHandle (tokio/src/task/)
- tokio/src/sync — synchronization primitives: Mutex, RwLock, channels, Semaphore (tokio/src/sync/)
- tokio/src/io — async I/O traits: AsyncRead, AsyncWrite, AsyncBufRead (tokio/src/io/)
- tokio/src/net — networking: TcpStream, TcpListener, UdpSocket (tokio/src/net/)
- tokio/src/fs — async filesystem operations (tokio/src/fs/)
- tokio/src/time — time driver: sleep, timeout, interval (tokio/src/time/)
- tokio/src/process — async process spawning (tokio/src/process/)
- tokio/src/signal — signal handling (tokio/src/signal/)
- tokio-macros — the #[tokio::main] and #[tokio::test] macros

## entrypoints
- #[tokio::main] — async main attribute macro (tokio-macros)
- tokio::runtime::Runtime — builder-created runtime entry
- Runtime::block_on — blocking entry into the async runtime
- tokio::spawn — spawn a task on the current runtime (tokio/src/task/spawn.rs)
- tokio::select! — multi-branch await macro
- tokio::join! — concurrent await macro
- tokio::sync::Mutex::lock — async mutex entry
- tokio::time::sleep — async sleep entry
- tokio::net::TcpListener::accept — accept loop entry
- tokio::io::AsyncWriteExt::write_all — async write entry

## behavior
- Runtime::new -> Builder::build -> create scheduler + driver — runtime construction (tokio/src/runtime/builder.rs)
- block_on -> enter runtime -> park driver loop — blocking entry (tokio/src/runtime/block_on.rs)
- spawn -> schedule task on worker queue -> worker poll loop — task execution (tokio/src/runtime/scheduler/)
- worker.poll -> poll_task -> run task future — worker loop (tokio/src/runtime/worker.rs)
- sleep -> register timer in time driver -> park until deadline — timer scheduling (tokio/src/time/driver/)
- TcpListener.accept -> poll_accept -> socket readiness via driver — I/O readiness (tokio/src/net/tcp/listener.rs)
- select! -> poll all branches -> winner continuation — branch selection
- blocking pool: spawn_blocking -> BlockingPool -> thread spawn — blocking offload (tokio/src/runtime/blocking/)

## state_authority
- Runtime — the runtime handle and state (tokio/src/runtime/runtime.rs)
- Scheduler — the task scheduler (multithreaded/current-thread variants)
- worker.current_task / queue — per-worker task queue state
- TimeDriver — the timer wheel (tokio/src/time/driver/wheel.rs)
- IO driver — readiness registry (tokio/src/runtime/io/)
- BlockingPool — the blocking thread pool (tokio/src/runtime/blocking/pool.rs)
- Handle — runtime handle shared across tasks

## contracts
- #[tokio::main] async fn main() — runtime main contract
- Runtime::new() — default runtime construction
- tokio::spawn(async { ... }) -> JoinHandle — task spawn contract
- Mutex::new(x).lock().await — async mutex contract
- mpsc::channel(n) — channel creation contract
- sleep(Duration).await — sleep contract
- timeout(dur, future).await — timeout contract
- TcpListener::bind(addr).await? -> accept().await — server contract
- select! { ... } — branch selection contract
- tokio::fs::read_to_string(path).await — fs contract

## landmarks
- Runtime — the runtime type (tokio/src/runtime/runtime.rs)
- Builder — runtime configuration builder (tokio/src/runtime/builder.rs)
- JoinHandle — task completion handle (tokio/src/task/join.rs)
- LocalSet — current-thread task set
- Worker — worker thread state (tokio/src/runtime/worker.rs)
- Wheel — timer wheel (tokio/src/time/driver/wheel.rs)
- BlockingPool — blocking thread pool (tokio/src/runtime/blocking/pool.rs)
- semaphore::Semaphore — async semaphore (tokio/src/sync/semaphore.rs)

## tests
- tokio/tests/ — integration tests (tokio/tests/rt_common.rs, tokio/tests/time.rs, etc.)
- tokio/tests/rt_common.rs — runtime behavior tests
- tokio/tests/time.rs — timer tests
- tokio/tests/sync_*.rs — sync primitive tests
- tokio/tests/io_*.rs — I/O tests
- tokio/tests/net_*.rs — networking tests
