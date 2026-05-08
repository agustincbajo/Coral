//! v0.20.2 audit-followup #37: end-to-end test for the MCP
//! `reviewed: false` distilled-page filter.
//!
//! The lint-based trust gate (`unreviewed-distilled` rule, v0.20.1
//! H2) blocks `reviewed: false` distilled pages from entering git
//! via the pre-commit hook. But before this fix, `coral mcp serve`
//! advertised and served them via `coral://wiki/_index` AND
//! `coral://wiki/<repo>/<slug>` immediately after `coral session
//! distill --apply` wrote them — i.e. attacker-influenced (via
//! prompt injection through the original transcript) content was
//! reachable by other agents BEFORE a human reviewer flipped
//! `reviewed: true`.
//!
//! Default behavior (this test): the unreviewed page is hidden from
//! `_index`, hidden from per-page enumeration, and unreadable via
//! its slug URI. With `--include-unreviewed` (debugging path), both
//! pages appear.

use coral_mcp::{ResourceProvider, WikiResourceProvider};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn build_fixture(root: &Path) {
    fs::create_dir_all(root.join(".wiki/modules")).unwrap();
    // Reviewed distilled page — should always be visible.
    fs::write(
        root.join(".wiki/modules/safe.md"),
        "---\n\
         slug: safe\n\
         type: module\n\
         last_updated_commit: deadbeef\n\
         confidence: 0.7\n\
         status: draft\n\
         reviewed: true\n\
         source:\n  runner: claude-sonnet-4-5\n\
         ---\n\n\
         # Safe\n\nReviewed distilled content.\n",
    )
    .unwrap();
    // UNreviewed distilled page — must be hidden by default.
    fs::write(
        root.join(".wiki/modules/risky.md"),
        "---\n\
         slug: risky\n\
         type: module\n\
         last_updated_commit: deadbeef\n\
         confidence: 0.7\n\
         status: draft\n\
         reviewed: false\n\
         source:\n  runner: claude-sonnet-4-5\n\
         ---\n\n\
         # Risky\n\nThis came straight from a distill — not yet reviewed.\n",
    )
    .unwrap();
    // Hand-authored draft with `reviewed: false` but NO source.runner —
    // the lint qualifier exempts these (v0.20.1 H2). Mirror that in
    // MCP: this page MUST stay visible.
    fs::write(
        root.join(".wiki/modules/handcrafted.md"),
        "---\n\
         slug: handcrafted\n\
         type: module\n\
         last_updated_commit: deadbeef\n\
         confidence: 0.7\n\
         status: draft\n\
         reviewed: false\n\
         ---\n\n\
         # Handcrafted draft\n\nHuman-authored, marked as not-yet-reviewed.\n",
    )
    .unwrap();
}

/// Default `WikiResourceProvider` (no `--include-unreviewed`):
/// `coral://wiki/_index` excludes the unreviewed distilled page.
#[test]
fn default_index_excludes_unreviewed_distilled() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    let (body, _mime) = p
        .read("coral://wiki/_index")
        .expect("aggregate index readable");
    let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    let pages = v["pages"].as_array().expect("pages array");
    let slugs: Vec<&str> = pages.iter().map(|p| p["slug"].as_str().unwrap()).collect();
    assert!(
        slugs.contains(&"safe"),
        "safe (reviewed) must appear: {slugs:?}"
    );
    assert!(
        slugs.contains(&"handcrafted"),
        "handcrafted (no source.runner) must appear: {slugs:?}"
    );
    assert!(
        !slugs.contains(&"risky"),
        "risky (reviewed: false + source.runner) must NOT appear by default: {slugs:?}"
    );
}

/// Default `WikiResourceProvider`: per-page resource list excludes
/// the unreviewed distilled page so MCP clients enumerating `list()`
/// don't even see the URI.
#[test]
fn default_list_excludes_unreviewed_distilled() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    let resources = p.list();
    let uris: Vec<&str> = resources.iter().map(|r| r.uri.as_str()).collect();
    assert!(
        uris.iter().any(|u| u.ends_with("/safe")),
        "safe (reviewed) URI must appear: {uris:?}"
    );
    assert!(
        uris.iter().any(|u| u.ends_with("/handcrafted")),
        "handcrafted (no source.runner) URI must appear: {uris:?}"
    );
    assert!(
        !uris.iter().any(|u| u.ends_with("/risky")),
        "risky URI must NOT appear by default: {uris:?}"
    );
}

/// Default `WikiResourceProvider`: even if a client guesses the URI,
/// `read("coral://wiki/risky")` returns `None` (renders as -32601 /
/// not found at the JSON-RPC layer).
#[test]
fn default_read_blocks_unreviewed_distilled_slug() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf());
    // `safe` reads fine.
    let (safe_body, _) = p
        .read("coral://wiki/safe")
        .expect("reviewed distilled page must read");
    let v: serde_json::Value = serde_json::from_str(&safe_body).unwrap();
    assert_eq!(v["slug"], "safe");
    // `risky` is not readable.
    assert!(
        p.read("coral://wiki/risky").is_none(),
        "unreviewed distilled page MUST NOT be readable by default"
    );
    // Handcrafted draft (no source.runner) IS readable.
    let (h_body, _) = p
        .read("coral://wiki/handcrafted")
        .expect("hand-authored draft must read");
    let v2: serde_json::Value = serde_json::from_str(&h_body).unwrap();
    assert_eq!(v2["slug"], "handcrafted");
}

/// `WikiResourceProvider::with_include_unreviewed(true)` (the
/// `--include-unreviewed` debug path) surfaces ALL pages, including
/// the unreviewed distilled one. Reading is also unblocked.
#[test]
fn include_unreviewed_flag_surfaces_all_pages() {
    let dir = TempDir::new().unwrap();
    build_fixture(dir.path());
    let p = WikiResourceProvider::new(dir.path().to_path_buf()).with_include_unreviewed(true);
    let (body, _) = p.read("coral://wiki/_index").unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let slugs: Vec<&str> = v["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["slug"].as_str().unwrap())
        .collect();
    assert!(slugs.contains(&"safe"));
    assert!(slugs.contains(&"risky"));
    assert!(slugs.contains(&"handcrafted"));
    // Reading the previously-blocked URI now succeeds.
    let (risky_body, _) = p.read("coral://wiki/risky").unwrap();
    let rv: serde_json::Value = serde_json::from_str(&risky_body).unwrap();
    assert_eq!(rv["slug"], "risky");
}
