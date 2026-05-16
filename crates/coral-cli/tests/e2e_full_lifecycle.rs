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

/// Initialise `repo` as a git repo with one commit so `git rev-parse HEAD`
/// succeeds. v0.34.0 cleanup: `coral init` no longer silently falls back
/// to a zero SHA in a non-git tempdir; tests must materialise a real HEAD.
fn git_init_with_commit(repo: &Path) {
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "lifecycle-test@coral.local"][..],
        &["config", "user.name", "Coral Lifecycle Test"][..],
        &["commit", "-q", "--allow-empty", "-m", "lifecycle fixture"][..],
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

#[test]
fn full_lifecycle_with_mock_runner() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().to_path_buf();
    let wiki = cwd.join(".wiki");

    let cur = std::env::current_dir().unwrap();
    std::env::set_current_dir(&cwd).unwrap();
    git_init_with_commit(&cwd);

    // Step 1: init.
    init::run(
        InitArgs {
            force: false,
            yes: false,
        },
        Some(&wiki),
    )
    .unwrap();
    assert!(wiki.join("SCHEMA.md").exists());
    assert!(wiki.join("index.md").exists());
    assert!(wiki.join("log.md").exists());
    assert!(wiki.join("modules").is_dir());
    assert!(wiki.join("concepts").is_dir());

    // Step 2: bootstrap with mock runner (responds with a YAML page list).
    std::fs::write(cwd.join("README.md"), "# repo").unwrap();
    let runner = MockRunner::new();
    // v0.34.0 (FR-ONB-30 refactor): bootstrap now parses the plan
    // even on the default-dry-run path so the planner output gets
    // a YAML schema check on every call. The fixture was a bare
    // sequence in v0.33; rewritten as a `plan:` mapping per the
    // bootstrap template contract.
    runner.push_ok(
        "plan:\n  - slug: order\n    type: module\n    rationale: top-level entity\n  - slug: outbox\n    type: concept\n    rationale: pattern used for delivery",
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

    // Step 4: ingest with mock runner (dry-run: just confirms the pipeline).
    let runner2 = MockRunner::new();
    runner2.push_ok(
        "plan:\n  - slug: order\n    action: update\n    rationale: handler signature changed",
    );
    ingest::run_with_runner(
        IngestArgs {
            from: Some("abc".into()),
            model: None,
            provider: None,
            dry_run: true,
            apply: false,
            ..Default::default()
        },
        Some(&wiki),
        &runner2,
    )
    .unwrap();
    assert_eq!(runner2.calls().len(), 1);
    assert!(runner2.calls()[0].user.contains("abc.."));

    // Step 4b: ingest --apply mutates the existing page (bumps last_updated_commit).
    let runner3 = MockRunner::new();
    runner3.push_ok(
        "plan:\n  - slug: order\n    action: update\n    rationale: handler signature changed",
    );
    ingest::run_with_runner(
        IngestArgs {
            from: Some("abc".into()),
            model: None,
            provider: None,
            dry_run: false,
            apply: true,
            ..Default::default()
        },
        Some(&wiki),
        &runner3,
    )
    .unwrap();
    let order_md = std::fs::read_to_string(wiki.join("modules/order.md")).unwrap();
    assert!(
        !order_md.contains("last_updated_commit: abc123"),
        "ingest --apply should have bumped last_updated_commit on order.md"
    );

    // Step 4c: bootstrap --apply writes a brand-new page from a YAML plan.
    let runner_b = MockRunner::new();
    runner_b.push_ok(
        "plan:\n  - slug: payments\n    type: module\n    confidence: 0.7\n    rationale: new feature\n    body: |\n      # Payments\n",
    );
    bootstrap::run_with_runner(
        BootstrapArgs {
            apply: true,
            ..Default::default()
        },
        Some(&wiki),
        &runner_b,
    )
    .unwrap();
    assert!(
        wiki.join("modules/payments.md").exists(),
        "bootstrap --apply should have written modules/payments.md"
    );

    // Step 5: structural lint passes.
    lint::run(
        LintArgs {
            structural: true,
            semantic: false,
            all: false,
            format: "markdown".into(),
            provider: None,
            staged: false,
            auto_fix: false,
            apply: false,
            severity: "all".into(),
            rule: vec![],
            fix: false,
            suggest_sources: false,
            check_injection: false,
            no_check_injection: false,
            governance: false,
        },
        Some(&wiki),
    )
    .unwrap();

    // Step 6: stats prints (we just confirm it doesn't error).
    stats::run(
        StatsArgs {
            format: "markdown".into(),
            symbols: false,
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
    git_init_with_commit(&cwd);

    init::run(
        InitArgs {
            force: false,
            yes: false,
        },
        Some(&wiki),
    )
    .unwrap();
    write_page(
        &wiki.join("modules/payments.md"),
        "payments",
        "module",
        0.9,
        "reviewed",
        "Payments module.",
    );

    // Re-run init — should be a no-op for the seeded page.
    init::run(
        InitArgs {
            force: false,
            yes: false,
        },
        Some(&wiki),
    )
    .unwrap();
    let body = std::fs::read_to_string(wiki.join("modules/payments.md")).unwrap();
    assert!(body.contains("Payments module."));

    // Lint still passes.
    lint::run(
        LintArgs {
            structural: true,
            semantic: false,
            all: false,
            format: "markdown".into(),
            provider: None,
            staged: false,
            auto_fix: false,
            apply: false,
            severity: "all".into(),
            rule: vec![],
            fix: false,
            suggest_sources: false,
            check_injection: false,
            no_check_injection: false,
            governance: false,
        },
        Some(&wiki),
    )
    .unwrap();

    std::env::set_current_dir(&cur).unwrap();
}

/// Regression: `coral init` on a repo that already has `.wiki/SCHEMA.md`
/// must STILL apply the FR-ONB-34 security-critical `.gitignore`
/// entries and the FR-ONB-25 `CLAUDE.md` scaffold. Pre-fix, the
/// function short-circuited at the wiki-exists check and skipped every
/// post-wiki step, leaving repos that upgraded from a pre-v0.34 binary
/// without `.coral/` in their root `.gitignore` (which contains the
/// Anthropic API key in `.coral/config.toml`).
#[test]
fn init_rerun_applies_security_gitignore_when_wiki_already_exists() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().to_path_buf();
    let wiki = cwd.join(".wiki");
    let cur = std::env::current_dir().unwrap();
    std::env::set_current_dir(&cwd).unwrap();
    git_init_with_commit(&cwd);

    // Simulate a repo initialised by a pre-v0.34 binary: `.wiki/SCHEMA.md`
    // is present, but the root `.gitignore` does NOT have the
    // FR-ONB-34 security entries, and `CLAUDE.md` does not yet exist.
    std::fs::create_dir_all(&wiki).unwrap();
    std::fs::write(wiki.join("SCHEMA.md"), "# stale schema").unwrap();
    let gitignore = cwd.join(".gitignore");
    std::fs::write(&gitignore, "target/\nnode_modules/\n").unwrap();
    assert!(!cwd.join("CLAUDE.md").exists());

    init::run(
        InitArgs {
            force: false,
            yes: false,
        },
        Some(&wiki),
    )
    .unwrap();

    // FR-ONB-34: security-critical entries are now present.
    let gi = std::fs::read_to_string(&gitignore).unwrap();
    assert!(gi.contains(".coral/"), "missing `.coral/` entry: {gi}");
    assert!(
        gi.contains(".wiki/.bootstrap-state.json"),
        "missing bootstrap-state entry: {gi}"
    );
    assert!(
        gi.contains(".wiki/.bootstrap.lock"),
        "missing bootstrap.lock entry: {gi}"
    );
    // User's existing lines survive.
    assert!(gi.contains("target/"));
    assert!(gi.contains("node_modules/"));

    // FR-ONB-25: CLAUDE.md was scaffolded.
    let claude_md = std::fs::read_to_string(cwd.join("CLAUDE.md")).unwrap();
    assert!(
        claude_md.contains("## Coral routing"),
        "CLAUDE.md must contain the routing section: {claude_md}"
    );

    // Pre-existing wiki content was NOT clobbered (SCHEMA.md still
    // reads the stale fixture, not the embedded template).
    let schema_after = std::fs::read_to_string(wiki.join("SCHEMA.md")).unwrap();
    assert_eq!(
        schema_after, "# stale schema",
        "wiki re-run must not overwrite SCHEMA.md without --force"
    );

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
    git_init_with_commit(&cwd);

    init::run(
        InitArgs {
            force: false,
            yes: false,
        },
        Some(&wiki),
    )
    .unwrap();
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
            provider: None,
            staged: false,
            auto_fix: false,
            apply: false,
            severity: "all".into(),
            rule: vec![],
            fix: false,
            suggest_sources: false,
            check_injection: false,
            no_check_injection: false,
            governance: false,
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
