//! Template bundle validation tests (Phase F).
//!
//! Verifies that the embedded template files have valid frontmatter and required sections.

use std::path::PathBuf;

fn template_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../template")
}

fn read_md(rel_path: &str) -> String {
    let p = template_dir().join(rel_path);
    std::fs::read_to_string(&p).unwrap_or_else(|_| panic!("missing {}", p.display()))
}

#[test]
fn all_subagents_have_required_frontmatter() {
    for name in [
        "wiki-bibliotecario",
        "wiki-linter",
        "wiki-consolidator",
        "wiki-onboarder",
    ] {
        let content = read_md(&format!("agents/{name}.md"));
        assert!(content.starts_with("---\n"), "{name} missing leading ---");
        for field in ["name:", "description:", "tools:", "model:"] {
            assert!(content.contains(field), "{name} missing field {field}");
        }
    }
}

#[test]
fn all_slash_commands_present() {
    for name in ["wiki-ingest", "wiki-query", "wiki-lint", "wiki-onboard"] {
        let content = read_md(&format!("commands/{name}.md"));
        assert!(
            content.starts_with("---\n"),
            "{name} command missing frontmatter"
        );
        assert!(
            content.contains("description:"),
            "{name} command missing description"
        );
    }
}

#[test]
fn all_prompt_templates_present_with_substitutions() {
    for name in ["ingest", "query", "lint-semantic", "consolidate"] {
        let content = read_md(&format!("prompts/{name}.md"));
        assert!(
            content.contains("{{"),
            "{name} prompt has no {{var}} placeholder"
        );
    }
}

#[test]
fn schema_base_has_required_sections() {
    let content = read_md("schema/SCHEMA.base.md");
    for section in [
        "## Your role",
        "## Operations",
        "## Page types",
        "## Required frontmatter",
        "## Rules of gold",
    ] {
        assert!(
            content.contains(section),
            "SCHEMA.base.md missing section: {section}"
        );
    }
}

#[test]
fn schema_base_documents_wikilink_slug_convention() {
    let content = read_md("schema/SCHEMA.base.md");
    assert!(
        content.contains("## Wikilinks"),
        "SCHEMA.base.md missing `## Wikilinks` section"
    );
    assert!(
        content.contains("slug literally"),
        "SCHEMA.base.md `## Wikilinks` section must spell out the convention with the phrase \"slug literally\""
    );
}

#[test]
fn composite_actions_present_with_required_keys() {
    let actions_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.github/actions");
    for action in ["ingest", "lint", "consolidate"] {
        let path = actions_root.join(action).join("action.yml");
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("missing composite action {}", path.display()));
        let parsed: serde_yaml_ng::Value = serde_yaml_ng::from_str(&content)
            .unwrap_or_else(|e| panic!("action {action} YAML invalid: {e}"));
        let map = parsed
            .as_mapping()
            .unwrap_or_else(|| panic!("action {action} root must be mapping"));
        for key in ["name", "description", "inputs", "runs"] {
            assert!(
                map.contains_key(serde_yaml_ng::Value::from(key)),
                "action {action} missing key: {key}"
            );
        }
        // Composite uses
        let runs = map
            .get(serde_yaml_ng::Value::from("runs"))
            .unwrap()
            .as_mapping()
            .unwrap();
        assert_eq!(
            runs.get(serde_yaml_ng::Value::from("using"))
                .and_then(|v| v.as_str()),
            Some("composite"),
            "action {action} must use composite"
        );
    }
}

#[test]
fn hermes_validator_subagent_present() {
    let content = read_md("agents/wiki-validator.md");
    assert!(content.starts_with("---\n"));
    for field in ["name:", "description:", "tools:", "model:"] {
        assert!(content.contains(field), "missing {field}");
    }
}

#[test]
fn hermes_validate_composite_action_present() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.github/actions/validate/action.yml");
    let content =
        std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing {}", path.display()));
    let parsed: serde_yaml_ng::Value = serde_yaml_ng::from_str(&content).unwrap();
    let map = parsed.as_mapping().expect("root mapping");
    for key in ["name", "description", "inputs", "runs"] {
        assert!(map.contains_key(serde_yaml_ng::Value::from(key)));
    }
    let runs = map
        .get(serde_yaml_ng::Value::from("runs"))
        .unwrap()
        .as_mapping()
        .unwrap();
    assert_eq!(
        runs.get(serde_yaml_ng::Value::from("using"))
            .and_then(|v| v.as_str()),
        Some("composite")
    );
}

#[test]
fn workflow_yaml_parses() {
    let content = read_md("workflows/wiki-maintenance.yml");
    // YAML 1.1 quirk: GitHub Actions `on:` key can deserialize as boolean `true`
    // depending on the parser. serde_yaml_ng (YAML 1.2) keeps it as the string
    // "on", but we intentionally do NOT depend on that — we assert structural
    // integrity through the `jobs:` mapping, which is unambiguous.
    let parsed: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&content).expect("workflow YAML must parse");
    let mapping = parsed.as_mapping().expect("YAML root must be mapping");
    let jobs = mapping
        .get(serde_yaml_ng::Value::from("jobs"))
        .expect("workflow must have `jobs` key");
    let jobs_map = jobs.as_mapping().expect("jobs must be mapping");
    for job in ["ingest", "lint-semantic", "consolidate"] {
        assert!(
            jobs_map.contains_key(serde_yaml_ng::Value::from(job)),
            "missing job: {job}"
        );
    }
    // Belt-and-suspenders: the literal `name:` and `jobs:` lines must be present
    // in the source so a future refactor can't silently delete the workflow header.
    assert!(content.contains("name: Wiki maintenance"));
    assert!(content.contains("jobs:"));
}
