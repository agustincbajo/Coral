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

/// v0.34.0 cleanup B2: `coral init` requires a real git HEAD. Seed
/// every snapshot tempdir with `git init` + an empty commit so the
/// underlying `git rev-parse HEAD` resolves deterministically.
fn git_init_with_commit(repo: &Path) {
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "snapshot-test@coral.local"][..],
        &["config", "user.name", "Coral Snapshot Test"][..],
        &["commit", "-q", "--allow-empty", "-m", "snapshot fixture"][..],
    ] {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("git invocation failed");
        assert!(
            status.success(),
            "git {args:?} failed in {}",
            repo.display()
        );
    }
}

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
/// etc. — matching what a real user's wiki would look like. v0.34.0
/// week 3 validator B2: `coral init` now hard-fails outside a git
/// repo, so we materialise an empty `git` history first.
fn write_seed_wiki(root: &Path) {
    git_init_with_commit(root);
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
        // 40-char hex commit SHAs (lower-case). v0.34.0 cleanup B2:
        // `coral init` now writes a real `git rev-parse HEAD` instead
        // of the deterministic zero SHA, so the snapshots see a
        // commit hash that changes every run. Filter both the real
        // SHA and the legacy zero SHA to `[COMMIT_SHA]`. The seed
        // page bodies hard-code `1234...5678` which would also match;
        // they're stable so the filter normalises them too.
        (r"\b[0-9a-f]{40}\b", "[COMMIT_SHA]"),
        // Tempdir prefix that may sneak into error messages or paths.
        // Matches macOS (/private/var/folders/.../.tmpXXX), plain
        // /var/folders/.../.tmpXXX, and Linux (/tmp/.tmpXXX) where the
        // tempdir is the immediate child of /tmp with no intermediate
        // path component. `[^\s]*` (zero-or-more) handles both cases.
        (
            r"(?:/private)?/(?:var/folders|tmp)/[^\s]*\.tmp[A-Za-z0-9]*",
            "[TMPDIR]",
        ),
        // Windows tempdir, e.g. C:\Users\<user>\AppData\Local\Temp\.tmpXXXX.
        // Used by `tempfile::TempDir` on Windows CI runners. Match on
        // the `\AppData\Local\Temp\.tmp<chars>` suffix so we don't have
        // to enumerate every possible user-profile prefix.
        (
            r"[A-Z]:\\Users\\[^\\]+\\AppData\\Local\\Temp\\\.tmp[A-Za-z0-9]+",
            "[TMPDIR]",
        ),
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

// ---- export -------------------------------------------------------------

/// Pins the JSON export of the seed wiki. The shape (slug, type, status,
/// confidence, sources, backlinks, body) is the documented contract that
/// downstream consumers script against — silent regressions here would
/// break their pipelines.
#[test]
fn export_json_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["export", "--format", "json"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

/// Pins the markdown-bundle export — the simplest "all pages
/// concatenated" format. Catches regressions in page ordering and the
/// per-page header (`## <slug> (<type>) — _status: ..., confidence: ..._`).
#[test]
fn export_markdown_bundle_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["export", "--format", "markdown-bundle"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

/// Pins the HTML head — only the doctype + opening tags through the TOC,
/// not the full body. The full HTML changes when CSS or wikilink
/// rendering changes; pinning the head + TOC catches structural
/// regressions (page count, type-grouping, section ordering) without
/// fighting body rendering churn. Trims to first 60 lines.
#[test]
fn export_html_head_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["export", "--format", "html"]);
    let head: String = stdout.lines().take(60).collect::<Vec<_>>().join("\n");
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(head);
    });
}

// ---- prompts ------------------------------------------------------------

/// Pins `coral prompts list` against the embedded template bundle. The
/// 9 known prompts (bootstrap, ingest, query, lint-semantic, consolidate,
/// onboard, qa-pairs, lint-auto-fix, diff-semantic) and their resolution
/// source (Local / Embedded / Fallback) are user-facing — drift here
/// breaks the discoverability the prompt-loader registry exists for.
#[test]
fn prompts_list_against_embedded_bundle() {
    let tmp = TempDir::new().unwrap();
    // Don't write a seed wiki — `prompts list` doesn't require .wiki/.
    let stdout = run_coral(tmp.path(), &["prompts", "list"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

// ---- validate-pin --------------------------------------------------------

/// Pins the `coral validate-pin` "no pins file" path. Drop in any
/// directory without `.coral-pins.toml` and the command should be a
/// no-op success with a stderr hint.
#[test]
fn validate_pin_no_pins_file() {
    let tmp = TempDir::new().unwrap();
    let assert = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("validate-pin")
        .assert()
        .success();
    let output = assert.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stderr);
    });
}

// ---- lint additional shapes ---------------------------------------------

/// Pins `coral lint --severity critical --format json`. Empty when the
/// seed has no Critical issues. Catches regressions where the filter
/// silently leaks Warning entries through.
#[test]
fn lint_severity_critical_json_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(
        tmp.path(),
        &["lint", "--severity", "critical", "--format", "json"],
    );
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

/// Pins `coral lint --severity warning` markdown output. Should include
/// every issue the seed produces (all are Warning) — same shape as the
/// existing `lint_structural_4_page_seed` snapshot but worth pinning
/// independently to catch any future severity-filter logic drift.
#[test]
fn lint_severity_warning_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["lint", "--severity", "warning"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

/// Pins `coral lint --rule source-not-found` against the 4-page seed.
/// Three sources don't exist on disk → exactly 3 SourceNotFound issues
/// keep through the filter; the LowConfidence + OrphanPage on
/// `idempotency` get dropped.
#[test]
fn lint_rule_source_not_found_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["lint", "--rule", "source-not-found"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

/// Pins `coral lint --rule low-confidence --rule orphan-page` — OR
/// semantics: keeps both code types. Matches the 2 issues on
/// `idempotency` and drops the 3 SourceNotFound entries.
#[test]
fn lint_rule_two_codes_or_semantics_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(
        tmp.path(),
        &["lint", "--rule", "low-confidence", "--rule", "orphan-page"],
    );
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

/// Pins `coral lint --fix` (dry-run) against the 4-page seed. The
/// seed is intentionally clean for the no-LLM rule set: sources are
/// single-element (so `sort-sources` is a no-op), backlinks are
/// short and either empty or single-element, no wikilink spacing
/// noise, no trailing whitespace, and no slug whitespace. The
/// dry-run should therefore report "No fixes needed." beneath the
/// existing lint output. Pinning catches regressions in either the
/// seed or the rule pass that would silently change which pages
/// fire — and confirms the fix output is appended without
/// disturbing the lint render above it.
#[test]
fn lint_fix_dry_run_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["lint", "--structural", "--fix"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

// ---- status -------------------------------------------------------------

/// Pins `coral status` markdown output against the 4-page seed wiki.
/// The seed has 4 pages, 1 init log entry, several Warning lint issues
/// (LowConfidence + OrphanPage on `idempotency`, SourceNotFound on the
/// three sourced pages). Filters scrub the path/timestamp noise so the
/// snapshot is stable across machines.
#[test]
fn status_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["status"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

// ---- history ------------------------------------------------------------

/// Pins `coral history outbox` against the 4-page seed. The seed only
/// has the init log entry from `coral init`, which mentions "wiki" not
/// "outbox" — so the expected output is the documented "no entries
/// mention 'outbox'" line. Acts as a regression guard for the empty-
/// match branch's wording.
#[test]
fn history_outbox_4_page_seed() {
    let tmp = TempDir::new().unwrap();
    write_seed_wiki(tmp.path());
    let stdout = run_coral(tmp.path(), &["history", "outbox"]);
    insta::with_settings!({ filters => standard_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}
