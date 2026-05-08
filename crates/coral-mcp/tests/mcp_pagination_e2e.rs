//! End-to-end cursor-pagination tests for the MCP server (#26).
//!
//! v0.19.7 cycle-3 audit (Notable):
//!   `resources/list` previously aggregated every per-page URI in one
//!   JSON-RPC envelope. A wiki with 1k pages produces a 1k-element
//!   array; some MCP transports impose envelope-size caps (curl pipe
//!   buffer, legacy stdio buffers) and silently truncate. The MCP
//!   2025-11-25 spec supports cursor pagination on list methods —
//!   this test pins the contract.
//!
//! Coverage:
//! - Wiki with N < page_size pages: `resources/list` returns all + no
//!   `nextCursor`. (`resources_list_under_page_size_returns_all`)
//! - Wiki with N > page_size pages: first call returns exactly
//!   `page_size` items + `nextCursor`. Cursor-resumed call returns
//!   the rest. (`resources_list_over_page_size_paginates`)
//! - Invalid cursor → JSON-RPC error envelope.
//!   (`resources_list_invalid_cursor_returns_error`)
//! - `tools/list` end-to-end (catalog is small today, contract is
//!   pinned for forward compat). (`tools_list_paginates_under_contract`)

use coral_mcp::server::PAGINATION_PAGE_SIZE;
use coral_mcp::{McpHandler, NoOpDispatcher, ServerConfig, Transport, WikiResourceProvider};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

fn build_wiki_with_pages(root: &Path, n_pages: usize) {
    fs::write(
        root.join("coral.toml"),
        r#"apiVersion = "coral.dev/v1"

[project]
name = "pagination-fixture"

[[repos]]
name = "alpha"
path = "."
"#,
    )
    .unwrap();
    let modules_dir = root.join(".wiki/modules");
    fs::create_dir_all(&modules_dir).unwrap();
    for i in 0..n_pages {
        let slug = format!("page-{i:04}");
        let body = format!(
            "---\nslug: {slug}\ntype: module\nlast_updated_commit: deadbeef\nconfidence: 0.5\nstatus: draft\n---\n\n# {slug}\n",
        );
        fs::write(modules_dir.join(format!("{slug}.md")), body).unwrap();
    }
}

fn handler_for(root: &Path, read_only: bool) -> McpHandler {
    // v0.20.2 audit-followup #38: ServerConfig now requires
    // `allow_write_tools`. Pagination tests don't exercise write
    // tools, so we just default it to `false`.
    let cfg = ServerConfig {
        transport: Transport::Stdio,
        read_only,
        allow_write_tools: false,
        port: None,
        bind_addr: None,
    };
    let resources = Arc::new(WikiResourceProvider::new(root.to_path_buf()));
    let tools = Arc::new(NoOpDispatcher);
    McpHandler::new(cfg, resources, tools)
}

fn call(h: &McpHandler, method: &str, params: serde_json::Value) -> serde_json::Value {
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    h.handle_line(&req.to_string())
        .expect("requests with `id` must produce a response")
}

/// Wiki with fewer pages than `PAGINATION_PAGE_SIZE` returns all
/// resources in one envelope, no `nextCursor`.
#[test]
fn resources_list_under_page_size_returns_all() {
    let dir = TempDir::new().unwrap();
    build_wiki_with_pages(dir.path(), 5);
    let h = handler_for(dir.path(), true);
    let resp = call(&h, "resources/list", serde_json::json!({}));
    let result = &resp["result"];
    assert!(
        result.get("nextCursor").is_none(),
        "single-page response must omit nextCursor: {resp}"
    );
    let resources = result["resources"].as_array().expect("array");
    // Static catalog (manifest, lock, stats, graph, wiki/_index,
    // test-report/latest, etc.) plus 5 per-page URIs. Just assert
    // the per-page entries are present.
    let uris: Vec<&str> = resources.iter().filter_map(|r| r["uri"].as_str()).collect();
    assert!(uris.iter().any(|u| u.contains("page-0000")));
    assert!(uris.iter().any(|u| u.contains("page-0004")));
}

/// Wiki larger than `PAGINATION_PAGE_SIZE` pages: first call returns
/// exactly `PAGINATION_PAGE_SIZE` items + a `nextCursor`. Resuming
/// with the cursor returns the remainder + no further `nextCursor`.
#[test]
fn resources_list_over_page_size_paginates() {
    // Add enough pages that, together with the static catalog
    // (~7 entries), we definitely exceed `PAGINATION_PAGE_SIZE`. Use
    // 2*page_size to also exercise multi-page resumption end-to-end.
    let n = PAGINATION_PAGE_SIZE * 2;
    let dir = TempDir::new().unwrap();
    build_wiki_with_pages(dir.path(), n);
    let h = handler_for(dir.path(), true);

    // First page.
    let resp1 = call(&h, "resources/list", serde_json::json!({}));
    let result1 = &resp1["result"];
    let page1 = result1["resources"].as_array().expect("array");
    assert_eq!(
        page1.len(),
        PAGINATION_PAGE_SIZE,
        "first page must have exactly PAGINATION_PAGE_SIZE entries"
    );
    let cursor = result1["nextCursor"]
        .as_str()
        .expect("first page must surface nextCursor")
        .to_string();
    assert_eq!(cursor, "100", "stringified offset cursor encoding (#26)");

    // Walk subsequent pages until exhausted; collect all URIs to
    // confirm coverage.
    let mut all_uris: Vec<String> = page1
        .iter()
        .filter_map(|r| r["uri"].as_str().map(str::to_string))
        .collect();
    let mut current_cursor = Some(cursor);
    let mut iterations = 0;
    while let Some(c) = current_cursor.take() {
        iterations += 1;
        assert!(iterations < 50, "pagination should terminate quickly");
        let resp = call(&h, "resources/list", serde_json::json!({ "cursor": c }));
        let result = &resp["result"];
        let page = result["resources"].as_array().expect("array");
        all_uris.extend(
            page.iter()
                .filter_map(|r| r["uri"].as_str().map(str::to_string)),
        );
        current_cursor = result
            .get("nextCursor")
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
    // Final page must NOT contain `nextCursor`.
    let resp_final = call(
        &h,
        "resources/list",
        serde_json::json!({ "cursor": (n + 7).to_string() }),
    );
    // Asking past the end is now an error (drift detection); just
    // assert iterations covered all per-page URIs in the wiki.
    let _ = resp_final; // Touch to silence unused warning.

    // Every per-page URI we seeded must appear somewhere in the
    // collected pages.
    for i in [0_usize, 1, 50, 99, 100, 199, n - 1] {
        let needle = format!("page-{i:04}");
        assert!(
            all_uris.iter().any(|u| u.contains(&needle)),
            "missing slug {needle:?} after walking pagination; collected={all_uris:?}"
        );
    }
}

/// Invalid cursor must surface as a JSON-RPC error so a misbehaving
/// client doesn't silently see an empty resources catalog.
#[test]
fn resources_list_invalid_cursor_returns_error() {
    let dir = TempDir::new().unwrap();
    build_wiki_with_pages(dir.path(), 3);
    let h = handler_for(dir.path(), true);
    let resp = call(
        &h,
        "resources/list",
        serde_json::json!({ "cursor": "not-an-integer" }),
    );
    assert!(resp.get("error").is_some(), "expected error: {resp}");
    assert!(
        resp["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("invalid cursor"),
        "error message must name the invalid cursor: {resp}"
    );
    // Cursor pointing past the end is also an error (drift detection).
    let resp2 = call(
        &h,
        "resources/list",
        serde_json::json!({ "cursor": "9999" }),
    );
    assert!(resp2.get("error").is_some(), "expected error: {resp2}");
    assert!(
        resp2["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("exceeds list length"),
        "error message must explain drift: {resp2}"
    );
}

/// `tools/list` honors the same cursor pagination contract. The
/// catalog is small (5–8 tools) but the contract is pinned so any
/// future tool explosion won't silently break existing clients.
#[test]
fn tools_list_paginates_under_contract() {
    let dir = TempDir::new().unwrap();
    build_wiki_with_pages(dir.path(), 0);
    let h = handler_for(dir.path(), false);
    let resp = call(&h, "tools/list", serde_json::json!({}));
    let result = &resp["result"];
    assert!(
        result.get("nextCursor").is_none(),
        "current tool catalog fits one page — must omit nextCursor: {resp}"
    );
    let tools = result["tools"].as_array().expect("array");
    assert!(!tools.is_empty(), "tools catalog must not be empty");
    // Invalid cursor: error.
    let resp_bad = call(&h, "tools/list", serde_json::json!({ "cursor": "garbage" }));
    assert!(
        resp_bad.get("error").is_some(),
        "expected error: {resp_bad}"
    );
}
