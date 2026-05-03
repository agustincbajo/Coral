//! MCP `Tool` catalog — the callable functions the agent can invoke.
//!
//! v0.19 wave 1 ships the descriptors. Wave 2 wires them to the real
//! implementation (delegating to existing CLI commands).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub kind: ToolKind,
    pub input_schema_json: String,
    /// Read-only tools work in `--read-only` mode (the default). Write
    /// tools require `--allow-write-tools` per PRD risk #25.
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Query,         // LLM-augmented (delegates to coral query)
    Search,        // TF-IDF (delegates to coral search)
    FindBacklinks, // pure
    AffectedRepos, // delegates to filters --since
    Verify,        // delegates to coral verify
    RunTest,       // delegates to coral test (write tool)
    Up,            // delegates to coral up (write tool)
    Down,          // delegates to coral down (write tool)
}

pub struct ToolCatalog;

impl ToolCatalog {
    pub fn read_only() -> Vec<Tool> {
        vec![
            Tool {
                name: "query".into(),
                description: "Streamed LLM answer using the wiki as context. Cites slugs.".into(),
                kind: ToolKind::Query,
                input_schema_json: r#"{"type":"object","properties":{"q":{"type":"string"},"repo":{"type":"string"},"tag":{"type":"string"}},"required":["q"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "search".into(),
                description: "TF-IDF full-text search across the wiki. No LLM.".into(),
                kind: ToolKind::Search,
                input_schema_json: r#"{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "find_backlinks".into(),
                description: "Return all wiki slugs that link to a given slug.".into(),
                kind: ToolKind::FindBacklinks,
                input_schema_json: r#"{"type":"object","properties":{"slug":{"type":"string"}},"required":["slug"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "affected_repos".into(),
                description: "List repos changed since a given SHA, including downstream dependents.".into(),
                kind: ToolKind::AffectedRepos,
                input_schema_json: r#"{"type":"object","properties":{"since":{"type":"string"}},"required":["since"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "verify".into(),
                description: "Run liveness healthchecks against the running environment.".into(),
                kind: ToolKind::Verify,
                input_schema_json: r#"{"type":"object","properties":{"env":{"type":"string"}}}"#.into(),
                read_only: true,
            },
        ]
    }

    pub fn write() -> Vec<Tool> {
        vec![
            Tool {
                name: "run_test".into(),
                description: "Run a specific test case by id against the running environment.".into(),
                kind: ToolKind::RunTest,
                input_schema_json: r#"{"type":"object","properties":{"case_id":{"type":"string"}},"required":["case_id"]}"#.into(),
                read_only: false,
            },
            Tool {
                name: "up".into(),
                description: "Bring up the dev environment (compose up).".into(),
                kind: ToolKind::Up,
                input_schema_json: r#"{"type":"object","properties":{"env":{"type":"string"},"watch":{"type":"boolean"}}}"#.into(),
                read_only: false,
            },
            Tool {
                name: "down".into(),
                description: "Tear down the dev environment.".into(),
                kind: ToolKind::Down,
                input_schema_json: r#"{"type":"object","properties":{"env":{"type":"string"},"volumes":{"type":"boolean"}}}"#.into(),
                read_only: false,
            },
        ]
    }

    pub fn all() -> Vec<Tool> {
        let mut all = Self::read_only();
        all.extend(Self::write());
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_tools_are_all_marked_read_only() {
        for t in ToolCatalog::read_only() {
            assert!(t.read_only, "{} should be read-only", t.name);
        }
    }

    #[test]
    fn write_tools_are_not_marked_read_only() {
        for t in ToolCatalog::write() {
            assert!(!t.read_only, "{} should not be read-only", t.name);
        }
    }

    #[test]
    fn all_returns_full_catalog() {
        let all = ToolCatalog::all();
        assert_eq!(
            all.len(),
            ToolCatalog::read_only().len() + ToolCatalog::write().len()
        );
    }

    #[test]
    fn input_schemas_are_valid_json() {
        for t in ToolCatalog::all() {
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(&t.input_schema_json);
            assert!(
                parsed.is_ok(),
                "tool {} has invalid schema: {}",
                t.name,
                t.input_schema_json
            );
        }
    }
}
