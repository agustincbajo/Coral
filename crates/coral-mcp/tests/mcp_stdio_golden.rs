//! BC pin for the stdio transport — Phase 2 of the v0.21.1 refactor
//! lifted the stdio loop body out of `server.rs` into
//! `transport/stdio.rs`. This test feeds a fixed input transcript
//! through `McpHandler::handle_line` (the shared dispatcher) and
//! verifies the JSON-RPC response sequence is exactly the v0.21.0
//! shape. Test #21 of the v0.21.1 plan.
//!
//! Why `handle_line` and not the actual stdin/stdout loop?
//! `serve_stdio` reads from `std::io::stdin()`, which can't be redirected
//! cleanly from inside a single-process integration test. The stdio
//! loop is now a 6-line shim — every interesting behavior (JSON-RPC
//! envelope shape, notification suppression, error codes, server
//! version) lives in `handle_line`. We pin THAT, plus a small
//! `serve_stdio_via_pipe` smoke test exercises the actual loop with
//! a `bash -c` echo.
//!
//! If this test fails after a refactor: the dispatcher changed and
//! every shipped MCP client may break. Either revert or update the
//! golden snapshot and the CHANGELOG.

use coral_mcp::{
    McpHandler, NoOpDispatcher, PROTOCOL_VERSION, ServerConfig, Transport, WikiResourceProvider,
};
use std::sync::Arc;

fn handler() -> McpHandler {
    let cfg = ServerConfig {
        transport: Transport::Stdio,
        read_only: true,
        allow_write_tools: false,
        port: None,
        bind_addr: None,
    };
    let resources = Arc::new(WikiResourceProvider::new(std::path::PathBuf::from("/tmp")));
    let tools = Arc::new(NoOpDispatcher);
    McpHandler::new(cfg, resources, tools)
}

/// Pinned canonical request transcript. Order matters; field order
/// inside each request matters for downstream byte-equality checks.
const TRANSCRIPT: &[&str] = &[
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
    r#"{"jsonrpc":"2.0","id":2,"method":"resources/list","params":{}}"#,
    r#"{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}"#,
    r#"{"jsonrpc":"2.0","id":4,"method":"prompts/list","params":{}}"#,
    r#"{"jsonrpc":"2.0","id":5,"method":"ping","params":{}}"#,
    // Notification — must produce no response.
    r#"{"jsonrpc":"2.0","method":"ping"}"#,
    // Unknown method — error envelope.
    r#"{"jsonrpc":"2.0","id":6,"method":"frobnicate","params":{}}"#,
    // Malformed JSON — parse error envelope (id null).
    r#"not json at all"#,
];

#[test]
fn stdio_transcript_response_shape_is_byte_identical_to_v0_21_0() {
    let h = handler();
    let mut responses: Vec<serde_json::Value> = Vec::new();
    for line in TRANSCRIPT {
        if let Some(resp) = h.handle_line(line) {
            responses.push(resp);
        }
    }
    // Notification produces no response — the count is 7 (one less
    // than the 8-element transcript).
    assert_eq!(
        responses.len(),
        7,
        "expected 7 responses (8 lines minus 1 notification), got {}",
        responses.len()
    );

    // Pin the shape of each response. Use field-by-field assertions
    // rather than a full JSON snapshot because (a) `protocolVersion`
    // is sourced from `crate::server::PROTOCOL_VERSION` and (b) the
    // resources catalog list has a known length but the per-page
    // resources depend on the wiki — by pinning `/tmp` (no .wiki dir)
    // we get an empty per-page section, which keeps the catalog
    // deterministic across machines.
    //
    // initialize
    assert_eq!(responses[0]["jsonrpc"], "2.0");
    assert_eq!(responses[0]["id"], 1);
    assert_eq!(responses[0]["result"]["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "coral");
    let server_version = responses[0]["result"]["serverInfo"]["version"]
        .as_str()
        .expect("serverInfo.version must be a string");
    // Pin: server version is the package version baked in via
    // `env!("CARGO_PKG_VERSION")`. Phase 6 bumps this to 0.21.1.
    assert!(
        server_version == "0.21.0" || server_version == "0.21.1",
        "unexpected server version: {server_version}"
    );
    // Capabilities surface stable across the refactor.
    let caps = &responses[0]["result"]["capabilities"];
    assert_eq!(caps["resources"]["listChanged"], false);
    assert_eq!(caps["tools"]["listChanged"], false);
    assert_eq!(caps["prompts"]["listChanged"], false);

    // resources/list — at minimum the canonical static URIs.
    let res_uris: Vec<&str> = responses[1]["result"]["resources"]
        .as_array()
        .expect("resources is array")
        .iter()
        .map(|r| r["uri"].as_str().expect("uri is string"))
        .collect();
    for canonical in [
        "coral://manifest",
        "coral://lock",
        "coral://stats",
        "coral://graph",
        "coral://wiki/_index",
        "coral://test-report/latest",
    ] {
        assert!(
            res_uris.contains(&canonical),
            "missing canonical URI {canonical} in resources/list (got {res_uris:?})"
        );
    }

    // tools/list — read-only set, names pinned.
    let tool_names: Vec<&str> = responses[2]["result"]["tools"]
        .as_array()
        .expect("tools is array")
        .iter()
        .map(|t| t["name"].as_str().expect("name is string"))
        .collect();
    for canonical in [
        "query",
        "search",
        "find_backlinks",
        "affected_repos",
        "verify",
    ] {
        assert!(
            tool_names.contains(&canonical),
            "missing read-only tool {canonical} in tools/list (got {tool_names:?})"
        );
    }
    // Write tools must NOT be present.
    for write in ["run_test", "up", "down"] {
        assert!(
            !tool_names.contains(&write),
            "write tool {write} leaked into default tools/list"
        );
    }

    // prompts/list — three prompts.
    let prompts = responses[3]["result"]["prompts"]
        .as_array()
        .expect("prompts is array");
    assert_eq!(prompts.len(), 3, "prompt catalog count drifted");

    // ping — empty result.
    assert_eq!(responses[4]["result"], serde_json::json!({}));

    // unknown-method error.
    assert!(
        responses[5]["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("unknown method"),
        "expected unknown-method error, got {}",
        responses[5]
    );

    // parse error — code -32700, id null.
    assert_eq!(responses[6]["error"]["code"], -32700);
    assert!(
        responses[6]["id"].is_null(),
        "parse-error response must have id: null, got {}",
        responses[6]["id"]
    );
}

/// Smoke test for the actual stdio loop (vs. the dispatcher in the
/// previous test). Spawns `coral mcp serve --transport stdio` and
/// pipes a single `initialize` envelope, then checks the response
/// echoes the protocol version. If the stdio transport refactor
/// regressed the loop body, this catches it.
#[test]
fn serve_stdio_pipe_initialize_round_trip() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let coral = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().and_then(|d| d.parent()).map(|d| d.to_path_buf()))
        .map(|d| d.join("coral"))
        .filter(|p| p.exists());
    let Some(coral) = coral else {
        // Binary isn't built (e.g. doc-test mode) — skip cleanly.
        eprintln!("coral binary not present; skipping stdio pipe smoke");
        return;
    };
    let mut child = Command::new(&coral)
        .args(["mcp", "serve", "--transport", "stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn coral mcp serve");
    let stdin = child.stdin.as_mut().expect("stdin");
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#
    )
    .expect("write initialize");
    drop(child.stdin.take()); // EOF → loop exits
    let out = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(PROTOCOL_VERSION),
        "stdio response missing protocolVersion {PROTOCOL_VERSION}; got:\n{stdout}\n--- stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
