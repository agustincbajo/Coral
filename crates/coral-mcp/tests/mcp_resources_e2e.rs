//! End-to-end resource read tests for the MCP server.
//!
//! v0.19.5 audit C1: previously the `read()` method always returned
//! `None`, so MCP clients couldn't actually consume any resource even
//! though `list()` advertised six. This test asserts every advertised
//! URI returns non-empty content against a tmpdir-built fixture.

use coral_mcp::{ResourceProvider, WikiResourceProvider};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn build_fixture(root: &Path) {
    fs::write(
        root.join("coral.toml"),
        r#"apiVersion = "coral.dev/v1"

[project]
name = "fixture"

[[repos]]
name = "alpha"
path = "."
"#,
    )
    .unwrap();
    fs::create_dir_all(root.join(".wiki/modules")).unwrap();
    fs::write(
        root.join(".wiki/modules/order.md"),
        "---\nslug: order\ntype: module\nlast_updated_commit: deadbeef\nconfidence: 0.7\nstatus: draft\n---\n\n# Order\n\nOrders link to [[invoice]].\n",
    )
    .unwrap();
    fs::write(
        root.join(".wiki/modules/invoice.md"),
        "---\nslug: invoice\ntype: module\nlast_updated_commit: deadbeef\nconfidence: 0.7\nstatus: draft\n---\n\n# Invoice\n",
    )
    .unwrap();
}

#[test]
fn manifest_resource_returns_json() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    let (body, _mime) = p.read("coral://manifest").expect("manifest readable");
    let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(v["name"], "fixture");
}

#[test]
fn lock_resource_returns_json_even_without_lockfile() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    let (body, _mime) = p.read("coral://lock").expect("lock readable");
    let _: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
}

#[test]
fn graph_resource_lists_repos() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    let (body, _mime) = p.read("coral://graph").expect("graph readable");
    let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert!(v["nodes"].is_array());
}

#[test]
fn wiki_index_resource_lists_pages() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    let (body, _mime) = p.read("coral://wiki/_index").expect("wiki index readable");
    let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    let slugs: Vec<&str> = v["pages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["slug"].as_str())
        .collect();
    assert!(slugs.contains(&"order"));
    assert!(slugs.contains(&"invoice"));
}

#[test]
fn stats_resource_returns_report_json() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    let (body, _mime) = p.read("coral://stats").expect("stats readable");
    let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    // StatsReport has a `total_pages` field.
    assert!(v.is_object());
}

#[test]
fn test_report_resource_returns_placeholder_when_missing() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    let (body, _mime) = p
        .read("coral://test-report/latest")
        .expect("test-report readable");
    let _: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
}

#[test]
fn page_resource_returns_body_for_known_slug() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    let (body, _mime) = p
        .read("coral://wiki/order")
        .expect("page resource readable");
    let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(v["slug"], "order");
    assert!(v["body"].as_str().unwrap().contains("Orders link to"));
}

/// v0.19.5 audit C4: page URIs with `..` segments must be rejected.
#[test]
fn page_resource_rejects_path_traversal() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    assert!(p.read("coral://wiki/../etc/passwd").is_none());
    assert!(p.read("coral://wiki/api/..").is_none());
}

/// v0.19.5 audit C1: `list()` advertises both static catalog and per-page
/// resources for every page found in the wiki.
#[test]
fn list_includes_per_page_resources() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    let uris: Vec<String> = p.list().into_iter().map(|r| r.uri).collect();
    assert!(uris.iter().any(|u| u == "coral://wiki/order"));
    assert!(uris.iter().any(|u| u == "coral://wiki/invoice"));
}

/// v0.19.6 audit C1: every URI's `read()` mime type matches the
/// `mime_type` declared by `list()` for the same URI. Previously
/// `server.rs` hardcoded `text/markdown` for every read response,
/// silently mislabeling every JSON resource as markdown.
#[test]
fn read_mime_type_matches_list_catalog_for_every_uri() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    let listed: std::collections::BTreeMap<String, String> =
        p.list().into_iter().map(|r| (r.uri, r.mime_type)).collect();
    // Spot-check the JSON catalog: these MUST come back as
    // `application/json`, NOT `text/markdown`.
    for uri in &[
        "coral://manifest",
        "coral://lock",
        "coral://stats",
        "coral://graph",
        "coral://wiki/_index",
        "coral://test-report/latest",
    ] {
        let listed_mime = listed.get(*uri).unwrap_or_else(|| {
            panic!("URI `{uri}` missing from list() catalog");
        });
        assert_eq!(
            listed_mime, "application/json",
            "list() catalog declares wrong mime for {uri}: got {listed_mime}"
        );
        let (_body, read_mime) = p.read(uri).unwrap_or_else(|| {
            panic!("read() returned None for catalog URI `{uri}`");
        });
        assert_eq!(
            read_mime, "application/json",
            "read() returned wrong mime for {uri}: got {read_mime}; \
             expected to match list()-declared `application/json`"
        );
    }
}

/// v0.19.6 audit C1: per-page wiki resources must be tagged
/// `application/json` (the underlying `render_page` payload is JSON,
/// not raw markdown). Pinning here so the contract doesn't drift.
#[test]
fn read_mime_type_for_per_page_uri_is_application_json() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    let (_body, mime) = p
        .read("coral://wiki/order")
        .expect("page resource readable");
    assert_eq!(mime, "application/json");
}

/// v0.19.6 audit N4: the `<repo>` segment in `coral://wiki/<repo>/_index`
/// arrives over MCP from an untrusted client. Without validation it
/// would be reflected verbatim in the response's `repo` field — a
/// stepping stone for chained prompt-injection attacks. Repo segments
/// that don't pass the safe-filename allowlist must be rejected.
#[test]
fn repo_index_rejects_path_traversal_and_metas_in_repo_segment() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    // Percent-encoded slash → contains `%`, allowlist rejects.
    assert!(p.read("coral://wiki/..%2F/_index").is_none());
    // Plain `..` segment.
    assert!(p.read("coral://wiki/.. /_index").is_none());
    // Embedded slash in segment.
    assert!(p.read("coral://wiki/foo bar/_index").is_none());
    // Leading dot.
    assert!(p.read("coral://wiki/.hidden/_index").is_none());
    // Sanity: a clean repo segment still reaches the renderer.
    let (_body, _mime) = p
        .read("coral://wiki/alpha/_index")
        .expect("clean segment must render");
}
