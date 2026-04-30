//! E2E lifecycle test.
//!
//! Walks through a full Coral lifecycle on a temp dir:
//!   1. init — creates .wiki/{SCHEMA, index, log, dirs}.
//!   2. bootstrap — invokes runner with file listing; prints suggestions.
//!   3. (manually seed a couple of pages to simulate the bibliotecario applying)
//!   4. ingest — reads diff, prompts runner.
//!   5. lint structural — passes on a clean wiki.
//!   6. stats — prints summary.
//!
//! Uses `MockRunner` so no real `claude` is required.

use coral_cli::commands::{
    bootstrap::{self, BootstrapArgs},
    ingest::{self, IngestArgs},
    init::{self, InitArgs},
    lint::{self, LintArgs},
    stats::{self, StatsArgs},
};
use coral_runner::MockRunner;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

/// Serializes any test in this binary that mutates `current_dir`. Tests that
/// only read the workspace state can run in parallel; tests that walk the cwd
/// (bootstrap, ingest) must hold this lock.
static CWD_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn full_lifecycle_with_mock_runner() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().to_path_buf();
    let wiki = cwd.join(".wiki");

    let cur = std::env::current_dir().unwrap();
    std::env::set_current_dir(&cwd).unwrap();

    // Step 1: init.
    init::run(InitArgs { force: false }, Some(&wiki)).unwrap();
    assert!(wiki.join("SCHEMA.md").exists());
    assert!(wiki.join("index.md").exists());
    assert!(wiki.join("log.md").exists());
    assert!(wiki.join("modules").is_dir());
    assert!(wiki.join("concepts").is_dir());

    // Step 2: bootstrap with mock runner (responds with a YAML page list).
    std::fs::write(cwd.join("README.md"), "# repo").unwrap();
    let runner = MockRunner::new();
    runner.push_ok(
        "- slug: order\n  type: module\n  rationale: top-level entity\n- slug: outbox\n  type: concept\n  rationale: pattern used for delivery",
    );
    bootstrap::run_with_runner(BootstrapArgs::default(), Some(&wiki), &runner).unwrap();
    assert_eq!(runner.calls().len(), 1);
    let bootstrap_user = &runner.calls()[0].user;
    assert!(
        bootstrap_user.contains("README.md"),
        "bootstrap prompt should include the listed files"
    );

    // Step 3: seed pages manually to simulate post-bootstrap state.
    write_page(
        &wiki.join("modules/order.md"),
        "order",
        "module",
        0.85,
        "reviewed",
        "Order module — references the [[outbox]] concept.",
    );
    write_page(
        &wiki.join("concepts/outbox.md"),
        "outbox",
        "concept",
        0.80,
        "verified",
        "Pattern used to guarantee at-least-once delivery. See [[order]].",
    );

    // Step 4: ingest with mock runner.
    let runner2 = MockRunner::new();
    runner2.push_ok("- slug: order\n  action: update\n  rationale: handler signature changed");
    ingest::run_with_runner(
        IngestArgs {
            from: Some("abc".into()),
            model: None,
        },
        Some(&wiki),
        &runner2,
    )
    .unwrap();
    assert_eq!(runner2.calls().len(), 1);
    assert!(runner2.calls()[0].user.contains("abc.."));

    // Step 5: structural lint passes.
    lint::run(
        LintArgs {
            structural: true,
            semantic: false,
            all: false,
            format: "markdown".into(),
        },
        Some(&wiki),
    )
    .unwrap();

    // Step 6: stats prints (we just confirm it doesn't error).
    stats::run(
        StatsArgs {
            format: "markdown".into(),
        },
        Some(&wiki),
    )
    .unwrap();

    // Restore cwd
    std::env::set_current_dir(&cur).unwrap();
}

#[test]
fn lifecycle_init_idempotent_does_not_clobber_seeded_pages() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().to_path_buf();
    let wiki = cwd.join(".wiki");
    let cur = std::env::current_dir().unwrap();
    std::env::set_current_dir(&cwd).unwrap();

    init::run(InitArgs { force: false }, Some(&wiki)).unwrap();
    write_page(
        &wiki.join("modules/payments.md"),
        "payments",
        "module",
        0.9,
        "reviewed",
        "Payments module.",
    );

    // Re-run init — should be a no-op for the seeded page.
    init::run(InitArgs { force: false }, Some(&wiki)).unwrap();
    let body = std::fs::read_to_string(wiki.join("modules/payments.md")).unwrap();
    assert!(body.contains("Payments module."));

    // Lint still passes.
    lint::run(
        LintArgs {
            structural: true,
            semantic: false,
            all: false,
            format: "markdown".into(),
        },
        Some(&wiki),
    )
    .unwrap();

    std::env::set_current_dir(&cur).unwrap();
}

#[test]
fn lifecycle_lint_json_format_emits_valid_json() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().to_path_buf();
    let wiki = cwd.join(".wiki");
    let cur = std::env::current_dir().unwrap();
    std::env::set_current_dir(&cwd).unwrap();

    init::run(InitArgs { force: false }, Some(&wiki)).unwrap();
    write_page(
        &wiki.join("modules/clean.md"),
        "clean",
        "module",
        0.7,
        "reviewed",
        "A clean page with no broken links.",
    );

    // Just confirm the command runs without panic; correctness of the JSON itself
    // is exercised by coral-lint unit tests.
    lint::run(
        LintArgs {
            structural: true,
            semantic: false,
            all: false,
            format: "json".into(),
        },
        Some(&wiki),
    )
    .unwrap();

    std::env::set_current_dir(&cur).unwrap();
}

fn write_page(path: &Path, slug: &str, page_type: &str, confidence: f64, status: &str, body: &str) {
    let body_full = format!(
        "---\nslug: {slug}\ntype: {page_type}\nlast_updated_commit: abc123\nconfidence: {confidence}\nsources:\n  - src/{slug}.rs\nstatus: {status}\n---\n\n{body}\n"
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body_full).unwrap();
}
