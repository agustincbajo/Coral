//! Semantic diff for API contracts via external tools.
//!
//! v0.24: shells out to available tools to detect breaking changes:
//! - `oasdiff` for OpenAPI specs (REST)
//! - `buf` for Protocol Buffers (gRPC)
//! - `atlas` for database schema migrations
//!
//! Each tool is optional — if not installed, the check is skipped with
//! a warning. This keeps Coral zero-mandatory-dep for the happy path.

use std::path::Path;
use std::process::Command;

/// Result of a semantic diff between two spec versions.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticDiffResult {
    pub tool: &'static str,
    pub breaking_changes: Vec<BreakingChange>,
    pub warnings: Vec<String>,
    pub skipped: bool,
    pub skip_reason: Option<String>,
}

/// A single breaking change detected by the external tool.
#[derive(Debug, Clone, PartialEq)]
pub struct BreakingChange {
    pub severity: DiffSeverity,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSeverity {
    Error,
    Warning,
}

/// Check if a tool is available on PATH.
fn tool_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run oasdiff breaking-changes check between two OpenAPI spec files.
/// Returns None if oasdiff is not installed.
pub fn diff_openapi(base: &Path, revision: &Path) -> SemanticDiffResult {
    if !tool_available("oasdiff") {
        return SemanticDiffResult {
            tool: "oasdiff",
            breaking_changes: vec![],
            warnings: vec![],
            skipped: true,
            skip_reason: Some("oasdiff not found on PATH (install: brew install oasdiff)".into()),
        };
    }

    let output = Command::new("oasdiff")
        .args(["breaking", "--format", "json"])
        .arg("--base")
        .arg(base)
        .arg("--revision")
        .arg(revision)
        .output();

    match output {
        Ok(o) if o.status.success() || o.status.code() == Some(1) => {
            // oasdiff exits 0 for no breaking changes, 1 for breaking changes found
            let stdout = String::from_utf8_lossy(&o.stdout);
            let changes = parse_oasdiff_json(&stdout);
            SemanticDiffResult {
                tool: "oasdiff",
                breaking_changes: changes,
                warnings: vec![],
                skipped: false,
                skip_reason: None,
            }
        }
        Ok(o) => SemanticDiffResult {
            tool: "oasdiff",
            breaking_changes: vec![],
            warnings: vec![format!(
                "oasdiff exited with code {:?}: {}",
                o.status.code(),
                String::from_utf8_lossy(&o.stderr)
            )],
            skipped: false,
            skip_reason: None,
        },
        Err(e) => SemanticDiffResult {
            tool: "oasdiff",
            breaking_changes: vec![],
            warnings: vec![],
            skipped: true,
            skip_reason: Some(format!("failed to run oasdiff: {e}")),
        },
    }
}

/// Run buf breaking check between two proto directories.
pub fn diff_protobuf(base_dir: &Path, revision_dir: &Path) -> SemanticDiffResult {
    if !tool_available("buf") {
        return SemanticDiffResult {
            tool: "buf",
            breaking_changes: vec![],
            warnings: vec![],
            skipped: true,
            skip_reason: Some(
                "buf not found on PATH (install: brew install bufbuild/buf/buf)".into(),
            ),
        };
    }

    let output = Command::new("buf")
        .args(["breaking", "--against"])
        .arg(base_dir)
        .arg(revision_dir)
        .args(["--format", "json"])
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let changes = parse_buf_json(&stdout);
            SemanticDiffResult {
                tool: "buf",
                breaking_changes: changes,
                warnings: vec![],
                skipped: false,
                skip_reason: None,
            }
        }
        Err(e) => SemanticDiffResult {
            tool: "buf",
            breaking_changes: vec![],
            warnings: vec![],
            skipped: true,
            skip_reason: Some(format!("failed to run buf: {e}")),
        },
    }
}

/// Run atlas schema diff for database migrations.
pub fn diff_schema(base: &Path, revision: &Path) -> SemanticDiffResult {
    if !tool_available("atlas") {
        return SemanticDiffResult {
            tool: "atlas",
            breaking_changes: vec![],
            warnings: vec![],
            skipped: true,
            skip_reason: Some(
                "atlas not found on PATH (install: brew install ariga/tap/atlas)".into(),
            ),
        };
    }

    let output = Command::new("atlas")
        .args(["schema", "diff"])
        .arg("--from")
        .arg(format!("file://{}", base.display()))
        .arg("--to")
        .arg(format!("file://{}", revision.display()))
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // atlas outputs SQL diff; any output means changes exist
            let changes = if stdout.trim().is_empty() {
                vec![]
            } else {
                vec![BreakingChange {
                    severity: DiffSeverity::Warning,
                    path: revision.display().to_string(),
                    message: format!(
                        "schema changes detected:\n{}",
                        stdout.chars().take(500).collect::<String>()
                    ),
                }]
            };
            SemanticDiffResult {
                tool: "atlas",
                breaking_changes: changes,
                warnings: vec![],
                skipped: false,
                skip_reason: None,
            }
        }
        Ok(o) => SemanticDiffResult {
            tool: "atlas",
            breaking_changes: vec![],
            warnings: vec![format!(
                "atlas exited {:?}: {}",
                o.status.code(),
                String::from_utf8_lossy(&o.stderr)
            )],
            skipped: false,
            skip_reason: None,
        },
        Err(e) => SemanticDiffResult {
            tool: "atlas",
            breaking_changes: vec![],
            warnings: vec![],
            skipped: true,
            skip_reason: Some(format!("failed to run atlas: {e}")),
        },
    }
}

/// Parse oasdiff JSON output into breaking changes.
fn parse_oasdiff_json(json_str: &str) -> Vec<BreakingChange> {
    let parsed: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let mut changes = Vec::new();
    if let Some(arr) = parsed.as_array() {
        for item in arr {
            let severity = match item.get("level").and_then(|v| v.as_str()) {
                Some("error") => DiffSeverity::Error,
                _ => DiffSeverity::Warning,
            };
            let path = item
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let message = item
                .get("message")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("text").and_then(|v| v.as_str()))
                .unwrap_or("breaking change detected")
                .to_string();
            changes.push(BreakingChange {
                severity,
                path,
                message,
            });
        }
    }
    changes
}

/// Parse buf breaking output into breaking changes.
fn parse_buf_json(json_str: &str) -> Vec<BreakingChange> {
    let parsed: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => {
            // buf may output line-delimited JSON
            return json_str
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .map(|item| {
                    let path = item
                        .get("path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let message = item
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("breaking change")
                        .to_string();
                    BreakingChange {
                        severity: DiffSeverity::Error,
                        path,
                        message,
                    }
                })
                .collect();
        }
    };

    if let Some(arr) = parsed.as_array() {
        arr.iter()
            .map(|item| {
                let path = item
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let message = item
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("breaking change")
                    .to_string();
                BreakingChange {
                    severity: DiffSeverity::Error,
                    path,
                    message,
                }
            })
            .collect()
    } else if parsed.is_object() {
        // Single object (not wrapped in an array)
        let path = parsed
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let message = parsed
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("breaking change")
            .to_string();
        vec![BreakingChange {
            severity: DiffSeverity::Error,
            path,
            message,
        }]
    } else {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_available_returns_false_for_nonexistent() {
        assert!(!tool_available("coral_nonexistent_tool_xyz_12345"));
    }

    #[test]
    fn diff_openapi_skips_when_tool_missing() {
        // oasdiff might not be installed in CI
        let result = diff_openapi(Path::new("/tmp/a.yaml"), Path::new("/tmp/b.yaml"));
        if result.skipped {
            assert!(result.skip_reason.unwrap().contains("oasdiff"));
        }
        // If oasdiff IS installed, the files don't exist so it should warn
    }

    #[test]
    fn parse_oasdiff_json_empty_array() {
        let changes = parse_oasdiff_json("[]");
        assert!(changes.is_empty());
    }

    #[test]
    fn parse_oasdiff_json_with_items() {
        let json = r#"[{"level":"error","path":"/users","message":"endpoint removed"}]"#;
        let changes = parse_oasdiff_json(json);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].severity, DiffSeverity::Error);
        assert_eq!(changes[0].path, "/users");
        assert_eq!(changes[0].message, "endpoint removed");
    }

    #[test]
    fn parse_buf_json_line_delimited() {
        let json = r#"{"path":"user.proto","message":"field removed"}"#;
        let changes = parse_buf_json(json);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].message, "field removed");
    }
}
