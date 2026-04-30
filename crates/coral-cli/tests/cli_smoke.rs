use assert_cmd::Command;
use predicates::str::contains;
use tempfile::TempDir;

#[test]
fn version_flag_prints_version() {
    Command::cargo_bin("coral")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(contains("coral"));
}

#[test]
fn init_creates_wiki_structure() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("init")
        .assert()
        .success();
    assert!(tmp.path().join(".wiki/SCHEMA.md").exists());
    assert!(tmp.path().join(".wiki/index.md").exists());
    assert!(tmp.path().join(".wiki/log.md").exists());
    assert!(tmp.path().join(".wiki/modules").is_dir());
    assert!(tmp.path().join(".wiki/concepts").is_dir());
    assert!(tmp.path().join(".wiki/decisions").is_dir());
}

#[test]
fn init_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    for _ in 0..3 {
        Command::cargo_bin("coral")
            .unwrap()
            .current_dir(tmp.path())
            .arg("init")
            .assert()
            .success();
    }
    assert!(tmp.path().join(".wiki/SCHEMA.md").exists());
}

#[test]
fn lint_on_empty_wiki_succeeds() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("init")
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("lint")
        .assert()
        .success()
        .stdout(contains("No issues"));
}

#[test]
fn lint_without_init_fails() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("lint")
        .assert()
        .failure()
        .stderr(contains("wiki root not found"));
}

#[test]
fn stats_on_empty_wiki_runs() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("init")
        .assert()
        .success();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("stats")
        .assert()
        .success()
        .stdout(contains("Total pages: 0"));
}

#[test]
fn stats_json_emits_valid_json() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("init")
        .assert()
        .success();
    let assert = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["stats", "--format", "json"])
        .assert()
        .success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("stats --format json should emit valid json");
    assert_eq!(value["total_pages"], 0);
}

#[test]
fn sync_extracts_template() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("sync")
        .assert()
        .success();
    assert!(tmp.path().join(".coral-template-version").exists());
    // The embedded bundle currently ships SCHEMA.base.md.
    assert!(
        tmp.path().join("template/schema/SCHEMA.base.md").exists(),
        "expected SCHEMA.base.md to land in template/schema/"
    );
}

#[test]
fn search_returns_not_implemented_exit_2() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["search", "anything"])
        .assert()
        .code(2);
}

#[test]
fn bootstrap_stub_returns_exit_2() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("bootstrap")
        .assert()
        .code(2);
}

#[test]
fn ingest_stub_returns_exit_2() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("ingest")
        .assert()
        .code(2);
}

#[test]
fn lint_critical_issue_exits_1() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("init")
        .assert()
        .success();

    // Drop a page with a broken wikilink to trigger a critical lint issue.
    let modules = tmp.path().join(".wiki/modules");
    std::fs::create_dir_all(&modules).unwrap();
    std::fs::write(
        modules.join("a.md"),
        "---\nslug: a\ntype: module\nlast_updated_commit: abc\nconfidence: 0.5\nstatus: draft\n---\n\nSee [[nonexistent]]\n",
    )
    .unwrap();

    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("lint")
        .assert()
        .code(1);
}
