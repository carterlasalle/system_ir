//! Context7 real-protocol integration test (P0 §16): starts the ACTUAL
//! `@upstash/context7-mcp` server (via npx) and exercises the full MCP stdio
//! handshake + tool calls through the SCC adapter's own client — protocol
//! fakes in the adapter unit tests are not a substitute for compatibility
//! with the real server.
//!
//! Optional suite: requires network + npx. Run with:
//!
//! ```sh
//! SCC_CONTEXT7_LIVE=1 cargo test -p scc-cli --test context7_live -- --ignored
//! ```

use std::process::Command;

const SERVER_CMD: &str = "npx -y @upstash/context7-mcp";

fn live_enabled() -> bool {
    std::env::var("SCC_CONTEXT7_LIVE").map(|v| v == "1").unwrap_or(false)
}

#[test]
#[ignore = "needs network + npx; run with SCC_CONTEXT7_LIVE=1"]
fn real_context7_server_protocol_compatibility() {
    if !live_enabled() {
        eprintln!("skipping: set SCC_CONTEXT7_LIVE=1 to run the live Context7 suite");
        return;
    }
    // the server must actually be spawnable
    let probe = Command::new("sh")
        .arg("-c")
        .arg(format!("{SERVER_CMD} --help"))
        .output()
        .expect("spawn context7");
    assert!(
        probe.status.success(),
        "context7 server must launch: {}",
        String::from_utf8_lossy(&probe.stderr)
    );

    let mut client = scc_indexer::adapters::context7::start(SERVER_CMD)
        .expect("MCP handshake with the real server");

    // docs_for exercises: initialize, notifications/initialized, tools/call
    // (library-search + query-docs) over real Content-Length framing
    let docs = client
        .docs_for("react")
        .expect("library-search + query-docs must succeed against the real server");
    assert!(
        docs.contains("CONTEXT7 EXTERNAL DOCUMENTATION"),
        "labeled external docs marker: {docs}"
    );
    assert!(
        docs.to_ascii_lowercase().contains("react"),
        "docs must mention the library: {docs}"
    );
}

#[test]
#[ignore = "needs network + npx; run with SCC_CONTEXT7_LIVE=1"]
fn real_context7_server_rejects_unknown_library_gracefully() {
    if !live_enabled() {
        return;
    }
    let mut client = scc_indexer::adapters::context7::start(SERVER_CMD)
        .expect("MCP handshake with the real server");
    // an impossible library name must not crash the client or hang it
    let result = client.docs_for("this-library-does-not-exist-xyz-12345");
    // either a clean error or empty docs — never a hang (client has a
    // 30s timeout) and never a panic
    let _ = result;
}
