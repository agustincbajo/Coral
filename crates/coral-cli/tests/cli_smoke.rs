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
fn lint_json_emits_object_with_issues_array() {
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
        .args(["lint", "--format", "json"])
        .assert()
        .success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint --format json should emit valid json");
    assert!(value.is_object(), "lint json must be an object: {stdout}");
    assert!(
        value.get("issues").and_then(|v| v.as_array()).is_some(),
        "lint json must have an `issues` array: {stdout}"
    );
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
fn sync_creates_pins_toml() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("sync")
        .assert()
        .success();
    let pins_path = tmp.path().join(".coral-pins.toml");
    assert!(
        pins_path.exists(),
        "expected .coral-pins.toml to be created"
    );
    let content = std::fs::read_to_string(&pins_path).unwrap();
    assert!(
        content.contains("default ="),
        "expected `default = ...` line in TOML, got: {content}"
    );
}

#[test]
fn sync_pin_flag_adds_entry() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["sync", "--pin", "agents/x=v0.9.0"])
        .assert()
        .success();
    let content = std::fs::read_to_string(tmp.path().join(".coral-pins.toml")).unwrap();
    assert!(
        content.contains("\"agents/x\""),
        "expected pinned key in TOML, got: {content}"
    );
    assert!(
        content.contains("v0.9.0"),
        "expected pinned version in TOML, got: {content}"
    );
}

#[test]
fn sync_unpin_flag_removes_entry() {
    let tmp = TempDir::new().unwrap();
    // First, set the pin.
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["sync", "--pin", "agents/x=v0.9.0"])
        .assert()
        .success();
    let content = std::fs::read_to_string(tmp.path().join(".coral-pins.toml")).unwrap();
    assert!(content.contains("\"agents/x\""));
    // Now remove it.
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["sync", "--unpin", "agents/x"])
        .assert()
        .success();
    let content = std::fs::read_to_string(tmp.path().join(".coral-pins.toml")).unwrap();
    assert!(
        !content.contains("\"agents/x\""),
        "expected pin to be removed, got: {content}"
    );
}

#[test]
fn sync_remote_without_version_fails() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["sync", "--remote"])
        .assert()
        .failure()
        .stderr(contains("remote sync requires"));
}

/// Hits the network — requires `git` + reachable github.com. Marked ignored so
/// CI / cargo test doesn't run it by default.
#[test]
#[ignore]
fn sync_remote_clones_tag_and_lays_template() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["sync", "--remote", "--version", "v0.1.0"])
        .assert()
        .success();
    assert!(tmp.path().join("template").is_dir());
    let pins = std::fs::read_to_string(tmp.path().join(".coral-pins.toml")).unwrap();
    assert!(pins.contains("default = \"v0.1.0\""));
}

#[test]
fn search_with_init_returns_no_results() {
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
        .args(["search", "anything"])
        .assert()
        .success()
        .stdout(contains("No results"));
}

#[test]
fn bootstrap_without_wiki_fails() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("bootstrap")
        .assert()
        .failure()
        .stderr(contains("wiki root not found"));
}

#[test]
fn ingest_without_wiki_fails() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("ingest")
        .assert()
        .failure()
        .stderr(contains("wiki root not found"));
}

#[test]
fn query_without_wiki_fails() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["query", "anything"])
        .assert()
        .failure()
        .stderr(contains("wiki root not found"));
}

#[test]
fn consolidate_without_wiki_fails() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("consolidate")
        .assert()
        .failure()
        .stderr(contains("wiki root not found"));
}

#[test]
fn onboard_without_wiki_fails() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("onboard")
        .assert()
        .failure()
        .stderr(contains("wiki root not found"));
}

#[test]
fn prompts_list_runs() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["prompts", "list"])
        .assert()
        .success()
        .stdout(contains("query"));
}

#[test]
fn query_with_unknown_provider_fails() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["query", "x", "--provider", "openai"])
        .assert()
        .failure()
        .stderr(contains("unknown provider"));
}

#[test]
fn export_markdown_bundle_runs() {
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
        .args(["export", "--format", "markdown-bundle"])
        .assert()
        .success()
        .stdout(contains("# Wiki bundle"));
}

#[test]
fn export_unknown_format_fails() {
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
        .args(["export", "--format", "yaml-but-fake"])
        .assert()
        .failure()
        .stderr(contains("unknown format"));
}

#[test]
fn export_writes_to_file_when_out_set() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("init")
        .assert()
        .success();
    let out = tmp.path().join("dump.md");
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args([
            "export",
            "--format",
            "markdown-bundle",
            "--out",
            out.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(out.exists());
}

#[test]
fn notion_push_without_token_fails() {
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
        .args(["notion-push", "--database", "db-fake"])
        .env_remove("NOTION_TOKEN")
        .assert()
        .failure()
        .stderr(contains("NOTION_TOKEN"));
}

#[test]
fn notion_push_defaults_to_dry_run() {
    // v0.4: dry-run is the default. `--apply` is required to actually POST.
    // No flag → preview message + success exit, never invokes curl.
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
        .args(["notion-push", "--token", "fake", "--database", "db-fake"])
        .assert()
        .success()
        .stdout(contains("Would POST"))
        .stdout(contains("--apply"));
}

#[test]
fn lint_writes_cache_file() {
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
        .success();
    let cache = tmp.path().join(".wiki/.coral-cache.json");
    assert!(cache.exists(), "expected walk cache at {}", cache.display());
}

#[test]
fn init_writes_wiki_gitignore() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("init")
        .assert()
        .success();
    let gi = tmp.path().join(".wiki/.gitignore");
    assert!(gi.exists(), "expected .wiki/.gitignore at {}", gi.display());
    let content = std::fs::read_to_string(&gi).unwrap();
    assert!(
        content.contains(".coral-cache.json"),
        "expected .coral-cache.json in .gitignore: {content}"
    );
}

#[test]
fn search_with_unknown_engine_fails() {
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
        .args(["search", "x", "--engine", "fancy"])
        .assert()
        .failure()
        .stderr(contains("unknown engine"));
}

#[test]
fn search_embeddings_without_api_key_fails() {
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
        .args(["search", "x", "--engine", "embeddings"])
        .env_remove("VOYAGE_API_KEY")
        .assert()
        .failure()
        .stderr(contains("VOYAGE_API_KEY"));
}

#[test]
fn init_gitignore_includes_embeddings() {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("init")
        .assert()
        .success();
    let gi = std::fs::read_to_string(tmp.path().join(".wiki/.gitignore")).unwrap();
    assert!(gi.contains(".coral-cache.json"));
    assert!(gi.contains(".coral-embeddings.json"));
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

/// Build a tempdir wiki that surfaces a Critical issue (broken wikilink) AND
/// at least one Warning (orphan page — `a` has no inbound backlinks). Returns
/// the tempdir so callers can run `coral lint --severity ... --format json`
/// against it and assert which issues survive the filter.
fn fixture_with_critical_and_warning() -> TempDir {
    let tmp = TempDir::new().unwrap();
    Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .arg("init")
        .assert()
        .success();
    let modules = tmp.path().join(".wiki/modules");
    std::fs::create_dir_all(&modules).unwrap();
    // Page `a`: broken wikilink (Critical) AND nothing links to it (Warning:
    // orphan). Two issues from one page keeps the test focused.
    std::fs::write(
        modules.join("a.md"),
        "---\nslug: a\ntype: module\nlast_updated_commit: abc\nconfidence: 0.5\nstatus: draft\n---\n\nSee [[nonexistent]]\n",
    )
    .unwrap();
    tmp
}

fn parse_lint_json(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout).expect("lint --format json should emit valid json")
}

fn issues_array(json: &serde_json::Value) -> &Vec<serde_json::Value> {
    json.get("issues")
        .and_then(|v| v.as_array())
        .expect("lint json must have an `issues` array")
}

#[test]
fn lint_severity_critical_filters_to_critical_only() {
    let tmp = fixture_with_critical_and_warning();
    // `--severity critical` keeps only Critical issues; the report still
    // exits 1 because the broken-wikilink Critical survives the filter.
    let assert = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["lint", "--severity", "critical", "--format", "json"])
        .assert()
        .code(1);
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let json = parse_lint_json(&stdout);
    let issues = issues_array(&json);
    assert!(
        !issues.is_empty(),
        "expected at least one Critical issue: {stdout}"
    );
    for issue in issues {
        assert_eq!(
            issue.get("severity").and_then(|v| v.as_str()),
            Some("critical"),
            "non-Critical issue leaked through filter: {issue}"
        );
    }
}

#[test]
fn lint_severity_warning_keeps_critical_and_warning() {
    let tmp = fixture_with_critical_and_warning();
    let assert = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["lint", "--severity", "warning", "--format", "json"])
        .assert()
        .code(1);
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let json = parse_lint_json(&stdout);
    let issues = issues_array(&json);
    let has_critical = issues
        .iter()
        .any(|i| i.get("severity").and_then(|v| v.as_str()) == Some("critical"));
    let has_warning = issues
        .iter()
        .any(|i| i.get("severity").and_then(|v| v.as_str()) == Some("warning"));
    let has_info = issues
        .iter()
        .any(|i| i.get("severity").and_then(|v| v.as_str()) == Some("info"));
    assert!(
        has_critical,
        "Critical missing under --severity warning: {stdout}"
    );
    assert!(
        has_warning,
        "Warning missing under --severity warning: {stdout}"
    );
    assert!(!has_info, "Info leaked under --severity warning: {stdout}");
}

#[test]
fn lint_severity_all_keeps_every_issue() {
    let tmp = fixture_with_critical_and_warning();
    // Baseline (all issues) — must be a strict superset of the warning run.
    let assert_all = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["lint", "--severity", "all", "--format", "json"])
        .assert()
        .code(1);
    let stdout_all = String::from_utf8_lossy(&assert_all.get_output().stdout);
    let count_all = issues_array(&parse_lint_json(&stdout_all)).len();

    let assert_warn = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(tmp.path())
        .args(["lint", "--severity", "warning", "--format", "json"])
        .assert()
        .code(1);
    let stdout_warn = String::from_utf8_lossy(&assert_warn.get_output().stdout);
    let count_warn = issues_array(&parse_lint_json(&stdout_warn)).len();

    assert!(
        count_all >= count_warn,
        "`all` ({count_all}) must be >= `warning` ({count_warn})"
    );
}

#[test]
fn lint_severity_unknown_value_fails_with_helpful_error() {
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
        .args(["lint", "--severity", "bogus"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("bogus"),
        "stderr should echo the bad value: {stderr}"
    );
    assert!(
        stderr.contains("critical")
            && stderr.contains("warning")
            && stderr.contains("info")
            && stderr.contains("all"),
        "stderr should list every valid value: {stderr}"
    );
}
