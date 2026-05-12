//! E2E query test.
//!
//! Init → seed pages → query with `MockRunner` → assert prompt context contains slugs.

use coral_cli::commands::{
    init::{self, InitArgs},
    query::{self, QueryArgs},
};
use coral_runner::MockRunner;
use std::sync::Mutex;
use tempfile::TempDir;

/// `init` reads `current_dir` to seed the index `last_commit`. Hold this lock
/// to keep the cwd stable across tests in this binary.
static CWD_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn query_cycle_with_mock_runner() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = TempDir::new().unwrap();
    let wiki = tmp.path().join(".wiki");
    let cur = std::env::current_dir().unwrap();
    std::env::set_current_dir(tmp.path()).unwrap();

    init::run(InitArgs { force: false, yes: false }, Some(&wiki)).unwrap();

    // Seed two pages.
    let body = |slug: &str, ptype: &str, content: &str| {
        format!(
            "---\nslug: {slug}\ntype: {ptype}\nlast_updated_commit: abc\nconfidence: 0.8\nsources: []\nbacklinks: []\nstatus: reviewed\n---\n\n{content}\n"
        )
    };
    let modules = wiki.join("modules");
    std::fs::create_dir_all(&modules).unwrap();
    std::fs::write(
        modules.join("order.md"),
        body(
            "order",
            "module",
            "Create-order endpoint persists Order rows.",
        ),
    )
    .unwrap();
    let concepts = wiki.join("concepts");
    std::fs::create_dir_all(&concepts).unwrap();
    std::fs::write(
        concepts.join("outbox.md"),
        body("outbox", "concept", "Outbox pattern guarantees delivery."),
    )
    .unwrap();

    let runner = MockRunner::new();
    runner.push_ok("Order is created via POST /orders. [[order]]");

    query::run_with_runner(
        QueryArgs {
            question: "How is an order created?".into(),
            model: None,
            provider: None,
            expand_graph: 0,
            at: None,
            verify: false,
            hyde: false,
        },
        Some(&wiki),
        &runner,
    )
    .unwrap();

    let calls = runner.calls();
    std::env::set_current_dir(&cur).unwrap();

    assert_eq!(calls.len(), 1);
    let user = &calls[0].user;
    assert!(user.contains("How is an order created?"));
    assert!(user.contains("order"));
    assert!(user.contains("outbox"));
}

#[test]
fn query_propagates_runner_error() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = TempDir::new().unwrap();
    let wiki = tmp.path().join(".wiki");
    let cur = std::env::current_dir().unwrap();
    std::env::set_current_dir(tmp.path()).unwrap();

    init::run(InitArgs { force: false, yes: false }, Some(&wiki)).unwrap();
    let runner = MockRunner::new();
    runner.push_err(coral_runner::RunnerError::NotFound);

    let res = query::run_with_runner(
        QueryArgs {
            question: "anything".into(),
            model: None,
            provider: None,
            expand_graph: 0,
            at: None,
            verify: false,
            hyde: false,
        },
        Some(&wiki),
        &runner,
    );

    std::env::set_current_dir(&cur).unwrap();
    assert!(res.is_err(), "runner error must surface as anyhow::Error");
}
