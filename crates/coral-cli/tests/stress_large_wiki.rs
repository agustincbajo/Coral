//! Stress integration tests against a synthetic 200-page wiki.
//!
//! These tests build a deterministic 200-page wiki on disk, then drive the
//! `coral` binary against it to assert wall-clock budgets for the hot
//! commands (`lint`, `stats`, `search`, `status`, `export`).
//!
//! Every test in this binary is `#[ignore]` because:
//!   - building 200 pages and running the full binary takes ~1-5s per test;
//!   - they exercise wall-clock budgets that are sensitive to load on the
//!     host machine, so running them in a noisy CI sandbox by default
//!     would create flakes;
//!   - the goal is "run on demand when investigating perf regressions" via
//!     `cargo test -p coral-cli --test stress_large_wiki -- --ignored`.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const TOTAL_PAGES: usize = 200;
const PER_TYPE: usize = 50;
const PAGE_TYPES: [(&str, &str); 4] = [
    ("module", "modules"),
    ("concept", "concepts"),
    ("entity", "entities"),
    ("flow", "flows"),
];

/// Deterministic body generator. Targets ~300 chars and embeds two wikilinks
/// computed via a counter so the link graph is reproducible without any RNG
/// dependency. All targets are guaranteed to exist (mod TOTAL_PAGES).
fn body_for(idx: usize, total: usize) -> String {
    let link_a = (idx * 7 + 3) % total;
    let link_b = (idx * 13 + 11) % total;
    format!(
        "Page {idx} body content. Refers to [[page-{link_a:04}]] and [[page-{link_b:04}]]. \
         Filler text to bulk up the body for realistic search testing. The wiki \
         indexer should treat this prose like any other module description and \
         keep the per-page tokens around the typical word count seen in real \
         repositories so that TF-IDF and BM25 scoring exercise representative \
         document lengths."
    )
}

/// Pick the (type, dir) tuple for a given page index. Pages 0..49 are modules,
/// 50..99 are concepts, 100..149 are entities, 150..199 are flows.
fn type_for(idx: usize) -> (&'static str, &'static str) {
    PAGE_TYPES[idx / PER_TYPE]
}

/// Build the synthetic wiki under `wiki_root`. `coral init` is run first so
/// the directory layout (SCHEMA.md, index.md, log.md, modules/, concepts/, …)
/// already exists; we then drop 200 pages on top of it.
///
/// Mixes in two deterministic patterns of intentional issues so lint has work
/// to do but never collapses under a wall of noise:
///   - Every 5th page (20%) gets `sources: []`, exercising the
///     `HighConfidenceWithoutSources` warning when confidence >= 0.6.
///   - Every 20th page (5%) gets `confidence: 0.20`, tripping the Critical
///     `LowConfidence` rule (confidence < 0.3).
fn build_synthetic_wiki(cwd: &Path) {
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(cwd)
        .arg("init")
        .assert()
        .success();

    for idx in 0..TOTAL_PAGES {
        let (page_type, dir) = type_for(idx);
        let slug = format!("page-{idx:04}");
        let body = body_for(idx, TOTAL_PAGES);

        let confidence = if idx % 20 == 0 { 0.20 } else { 0.85 };
        let sources_block = if idx % 5 == 0 {
            // Empty sources — under high confidence this trips
            // HighConfidenceWithoutSources (warning).
            "sources: []".to_string()
        } else {
            format!("sources:\n  - src/{slug}.rs")
        };

        let frontmatter = format!(
            "---\nslug: {slug}\ntype: {page_type}\nlast_updated_commit: abc\n\
             confidence: {confidence}\n{sources_block}\nstatus: draft\n---\n\n{body}\n"
        );

        let dest = cwd.join(".wiki").join(dir).join(format!("{slug}.md"));
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(&dest, frontmatter).unwrap();
    }
}

/// Assert that `elapsed` is under `budget`, with a friendly diagnostic that
/// names the operation. Centralized so every test prints the same shape.
fn assert_under(elapsed: Duration, budget: Duration, label: &str) {
    assert!(
        elapsed < budget,
        "{label} took {elapsed:?} (budget {budget:?})"
    );
    eprintln!("[stress] {label}: {elapsed:?} (budget {budget:?})");
}

// ============================================================================
// Test scenarios
// ============================================================================

/// Ignored: builds 200 pages on disk and shells out to the `coral` binary;
/// runs in ~1-3s wall-clock and is meant for manual perf inspection rather
/// than every PR.
#[test]
#[ignore = "stress test — run with --ignored when investigating lint perf"]
fn stress_lint_structural_200_pages() {
    let tmp = TempDir::new().unwrap();
    build_synthetic_wiki(tmp.path());

    let start = Instant::now();
    let assert = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["lint", "--structural"])
        .assert();
    let elapsed = start.elapsed();

    // Critical issues exist by design (5% of pages have confidence < 0.3, which
    // trips LowConfidence Critical), so the binary exits 1. Accept either 0 or
    // 1 because the stress fixture's exact issue mix is not the assertion under
    // test — wall-clock is.
    let code = assert.get_output().status.code().unwrap_or(-1);
    assert!(
        code == 0 || code == 1,
        "lint exit code should be 0 or 1, got {code}"
    );

    assert_under(elapsed, Duration::from_secs(5), "lint --structural");
}

/// Ignored: stress fixture takes time to materialize; manual perf probe only.
#[test]
#[ignore = "stress test — run with --ignored when investigating stats perf"]
fn stress_stats_200_pages() {
    let tmp = TempDir::new().unwrap();
    build_synthetic_wiki(tmp.path());

    let start = Instant::now();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("stats")
        .assert()
        .success();
    let elapsed = start.elapsed();

    assert_under(elapsed, Duration::from_secs(1), "stats");
}

/// Ignored: stress fixture takes time to materialize; manual perf probe only.
#[test]
#[ignore = "stress test — run with --ignored when investigating search perf"]
fn stress_search_200_pages_tfidf() {
    let tmp = TempDir::new().unwrap();
    build_synthetic_wiki(tmp.path());

    let start = Instant::now();
    let assert = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["search", "page", "--engine", "tfidf"])
        .assert()
        .success();
    let elapsed = start.elapsed();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        !stdout.contains("No results"),
        "expected results for query 'page', stdout was: {stdout}"
    );

    assert_under(elapsed, Duration::from_secs(1), "search tfidf");
}

/// Ignored: stress fixture takes time to materialize; manual perf probe only.
#[test]
#[ignore = "stress test — run with --ignored when investigating search perf"]
fn stress_search_200_pages_bm25() {
    let tmp = TempDir::new().unwrap();
    build_synthetic_wiki(tmp.path());

    let start = Instant::now();
    let assert = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["search", "page", "--engine", "tfidf", "--algorithm", "bm25"])
        .assert()
        .success();
    let elapsed = start.elapsed();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        !stdout.contains("No results"),
        "expected results for query 'page', stdout was: {stdout}"
    );

    assert_under(elapsed, Duration::from_secs(1), "search bm25");
}

/// Ignored: stress fixture takes time to materialize; manual perf probe only.
#[test]
#[ignore = "stress test — run with --ignored when investigating status perf"]
fn stress_status_200_pages() {
    let tmp = TempDir::new().unwrap();
    build_synthetic_wiki(tmp.path());

    let start = Instant::now();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("status")
        .assert()
        .success();
    let elapsed = start.elapsed();

    assert_under(elapsed, Duration::from_secs(5), "status");
}

/// Ignored: stress fixture takes time to materialize; manual perf probe only.
#[test]
#[ignore = "stress test — run with --ignored when investigating export perf"]
fn stress_export_html_200_pages() {
    let tmp = TempDir::new().unwrap();
    build_synthetic_wiki(tmp.path());
    let out = tmp.path().join("export.html");

    let start = Instant::now();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["export", "--format", "html", "--out", out.to_str().unwrap()])
        .assert()
        .success();
    let elapsed = start.elapsed();

    let size = fs::metadata(&out).unwrap().len();
    assert!(
        size > 50_000,
        "html export should be > 50KB for 200 pages, was {size} bytes"
    );

    assert_under(elapsed, Duration::from_secs(5), "export html");
}

/// Ignored: stress fixture takes time to materialize; manual perf probe only.
#[test]
#[ignore = "stress test — run with --ignored when investigating export perf"]
fn stress_export_json_200_pages() {
    let tmp = TempDir::new().unwrap();
    build_synthetic_wiki(tmp.path());
    let out = tmp.path().join("export.json");

    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["export", "--format", "json", "--out", out.to_str().unwrap()])
        .assert()
        .success();

    let raw = fs::read_to_string(&out).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("export json should be parseable: {e}\n{raw}"));
    assert!(
        parsed.is_array() || parsed.is_object(),
        "export json should be a JSON array or object, got: {parsed:?}"
    );
}
