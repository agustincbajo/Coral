//! Snapshot tests for Coral CLI's deterministic markdown output.
//!
//! These pin the user-facing rendering of `stats`, `lint --structural`,
//! `search`, and `diff` against a frozen 4-page seed wiki (the same
//! "order / outbox / checkout-flow / idempotency" seed used by
//! [docs/TUTORIAL.md](../../../docs/TUTORIAL.md)). Each test boots a
//! tempdir, writes the seed pages by hand, runs the CLI subcommand via
//! `assert_cmd::Command::cargo_bin("coral")`, filters out
//! non-deterministic noise (paths, timestamps, ANSI codes), and compares
//! the result to a committed `.snap` file under `tests/snapshots/`.
//!
//! Why snapshot the markdown specifically: hand-written `contains(...)`
//! assertions in [`cli_smoke.rs`](cli_smoke.rs) catch shape, not layout.
//! These snapshots catch accidental regressions in spacing, ordering,
//! emoji, and formatting that would otherwise ship silently.
//!
//! Updating snapshots: when a deliberate output change lands, run
//! `INSTA_UPDATE=always cargo test --test snapshot_cli -p coral-cli`
//! (or `cargo insta review` if `cargo-insta` is installed) and commit
//! the new `*.snap` files alongside the source change.

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

/// Build the deterministic 4-page seed wiki used by every snapshot test.
///
/// Mirrors the seed in [docs/TUTORIAL.md](../../../docs/TUTORIAL.md). Two
/// pages are intentionally broken so `lint --structural` surfaces known
/// warnings:
/// - `idempotency` is a draft with `confidence: 0.5` and zero inbound
///   backlinks → triggers `LowConfidence` + `OrphanPage`.
/// - All four pages cite source paths that don't exist on disk →
///   triggers one `SourceNotFound` per page that has any sources.
///
/// `coral init` is run first so the wiki has a `SCHEMA.md`, gitignore,
/// etc. — matching what a real user's wiki would look like.
fn write_seed_wiki(root: &Path) {
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(root)
        .arg("init")
        .assert()
        .success();

    let modules = root.join(".wiki/modules");
    let concepts = root.join(".wiki/concepts");
    let flows = root.join(".wiki/flows");
    std::fs::create_dir_all(&modules).unwrap();
    std::fs::create_dir_all(&concepts).unwrap();
    std::fs::create_dir_all(&flows).unwrap();

    std::fs::write(
        modules.join("order.md"),
        "---\n\
slug: order\n\
type: module\n\
last_updated_commit: 1234567890abcdef1234567890abcdef12345678\n\
confidence: 0.85\n\
sources:\n\
  - src/order/handler.rs\n\
backlinks: [outbox]\n\
status: reviewed\n\
---\n\
\n\
# Order\n\
\n\
Order creation flow. Receives POST /orders, validates, persists via the\n\
[[outbox]] pattern, returns 202.\n\
\n\
See [[checkout-flow]] for end-to-end behavior.\n",
    )
    .unwrap();

    std::fs::write(
        concepts.join("outbox.md"),
        "---\n\
slug: outbox\n\
type: concept\n\
last_updated_commit: 1234567890abcdef1234567890abcdef12345678\n\
confidence: 0.9\n\
sources:\n\
  - src/outbox/dispatcher.rs\n\
backlinks: [order]\n\
status: verified\n\
---\n\
\n\
# Outbox pattern\n\
\n\
Guarantees at-least-once delivery by writing intent to a local outbox\n\
table inside the same database transaction as the business write. A\n\
background dispatcher polls the outbox and emits to the message bus.\n\
\n\
Used by [[order]] to publish OrderCreated events.\n",
    )
    .unwrap();

    std::fs::write(
        flows.join("checkout-flow.md"),
        "---\n\
slug: checkout-flow\n\
type: flow\n\
last_updated_commit: 1234567890abcdef1234567890abcdef12345678\n\
confidence: 0.7\n\
sources:\n\
  - src/checkout/saga.rs\n\
backlinks: []\n\
status: reviewed\n\
---\n\
\n\
# Checkout flow\n\
\n\
End-to-end: cart \u{2192} payment \u{2192} [[order]]. Spans 3 services. The order\n\
service uses [[outbox]] for the OrderCreated event downstream.\n",
    )
    .unwrap();

    std::fs::write(
        concepts.join("idempotency.md"),
        "---\n\
slug: idempotency\n\
type: concept\n\
last_updated_commit: 1234567890abcdef1234567890abcdef12345678\n\
confidence: 0.5\n\
sources: []\n\
backlinks: []\n\
status: draft\n\
---\n\
\n\
# Idempotency\n\
\n\
A re-submitted request must produce the same observable result as the\n\
first. Implemented via request-id deduplication.\n",
    )
    .unwrap();
}

/// Run a coral subcommand against the seeded tempdir and return stdout
/// as a `String` for snapshotting. Asserts the command exits successfully
/// (or with the given expected code).
fn run_coral(cwd: &Path, args: &[&str]) -> String {
    let assert = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(cwd)
        .args(args)
        .assert();
    let output = assert.get_output();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Common redaction filters used across every snapshot. Keeps tempdir
/// paths, ISO-8601 timestamps, and raw ANSI escapes from spuriously
/// invalidating committed `.snap` files.
///
/// We use `&str` patterns for `insta`'s `filters` setting (regex source,
/// replacement). Order matters only when patterns overlap; ours don't.
fn standard_filters() -> Vec<(&'static str, &'static str)> {
    vec![
        // Strip ANSI escape codes (color/formatting) — present only in
        // some terminal-detection paths but cheap to filter unconditionally.
        (r"\x1b\[[\d;]*m", ""),
        // ISO-8601 timestamps with optional fractional seconds and tz.
        (
            r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})",
            "[TIMESTAMP]",
        ),
        // Tempdir prefix that may sneak into error messages or paths.
        // Matches macOS (/var/folders/...), Linux (/tmp/...), and Windows-style.
        (r"/(?:var/folders|tmp)/[^\s]+\.tmp[A-Za-z0-9]*", "[TMPDIR]"),
        // Generic catch-all for tempdir-ish absolute paths anchored at .wiki.
        // Keeps the snapshots free of `/private/var/folders/.../.wiki/...`.
        (
            r"/(?:private/)?(?:var/folders|tmp)/[^\s]+/\.wiki",
            "[TMPDIR]/.wiki",
        ),
    ]
}

// ---- stats ---------------------------------------------------------------

/// Pins the human-facing markdown dashboard for `coral stats` against
/// the 4-page seed. Catches changes to layout, ordering, emoji, and
/// number formatting.
#[test]
fn stats_default_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["stats"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

/// Pins the JSON form of `coral stats` against the 4-page seed. Acts as
/// a contract test for downstream consumers (jq pipelines, dashboards)
/// that depend on the exact field shape and ordering.
#[test]
fn stats_json_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["stats", "--format", "json"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

// ---- lint ---------------------------------------------------------------

/// Pins the markdown `coral lint --structural` report against the seed.
/// The seed was deliberately constructed to surface a known set of
/// warnings (LowConfidence on idempotency, OrphanPage on idempotency,
/// SourceNotFound on each of the three sourced pages); this snapshot
/// pins the exact wording, ordering, and severity buckets.
#[test]
fn lint_structural_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["lint", "--structural"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

/// Pins the JSON form of `coral lint --structural` for the same seed.
/// Contract test: any change to `LintIssue` field naming or ordering
/// will invalidate this snapshot and surface in review.
#[test]
fn lint_json_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["lint", "--structural", "--format", "json"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

// ---- search --------------------------------------------------------------

/// Pins the TF-IDF (`tfidf`) ranking + snippet rendering for the query
/// "outbox dispatcher" against the seed. Snippets are deterministic per
/// the seed body text, so any change to the snippet window or score
/// formatting will be caught here.
#[test]
fn search_outbox_dispatcher_tfidf() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["search", "outbox dispatcher"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

/// Pins the BM25 ranking for the same query so we can spot scoring
/// changes (BM25 gives different absolute scores than TF-IDF on the
/// same corpus). Same ranking on this small seed; different magnitudes.
#[test]
fn search_outbox_dispatcher_bm25() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(
        tmp.path(),
        &["search", "outbox dispatcher", "--algorithm", "bm25"],
    );
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

// ---- diff ---------------------------------------------------------------

/// Pins the structural `coral diff order outbox` report. Catches
/// regressions in the frontmatter table, sources/wikilinks set
/// arithmetic, and section ordering.
#[test]
fn diff_order_outbox() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["diff", "order", "outbox"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}
