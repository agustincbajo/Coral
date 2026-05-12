//! Test coverage analysis: cross OpenAPI endpoints vs TestCases.
//!
//! Walks the project for OpenAPI specs (reusing `discover`), loads
//! existing test YAML files, and reports which endpoints have tests
//! and which are gaps.

use crate::discover::{self, DiscoveredCase};
use crate::error::TestResult;
use crate::spec::{TestCase, TestKind};
use crate::walk_tests;
use std::collections::BTreeSet;
use std::path::Path;

/// A single endpoint found in OpenAPI specs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Endpoint {
    pub method: String,
    pub path: String,
    pub spec_file: String,
}

/// Coverage report for the project.
#[derive(Debug, Clone)]
pub struct CoverageReport {
    /// Endpoints that have at least one matching TestCase.
    pub covered: Vec<Endpoint>,
    /// Endpoints with no matching TestCase.
    pub gaps: Vec<Endpoint>,
    /// Total endpoint count.
    pub total: usize,
    /// Coverage percentage (0.0 - 100.0).
    pub percent: f64,
}

/// Compute test coverage by comparing OpenAPI endpoints against test cases.
pub fn compute_coverage(project_root: &Path) -> TestResult<CoverageReport> {
    // Discover all endpoints from OpenAPI specs
    let discovered = discover::discover_openapi_in_project(project_root)?;

    // Load existing test cases from .coral/tests/
    let existing_cases = load_existing_test_cases(project_root);

    // Build a set of (method, path) pairs that have tests
    let tested_endpoints: BTreeSet<(String, String)> = existing_cases
        .iter()
        .filter_map(extract_endpoint_from_case)
        .collect();

    // All unique endpoints from specs
    let all_endpoints: Vec<Endpoint> = discovered
        .iter()
        .map(endpoint_from_discovered)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut covered = Vec::new();
    let mut gaps = Vec::new();

    for ep in &all_endpoints {
        let key = (ep.method.to_lowercase(), ep.path.clone());
        if tested_endpoints.contains(&key) {
            covered.push(ep.clone());
        } else {
            gaps.push(ep.clone());
        }
    }

    let total = all_endpoints.len();
    let percent = if total > 0 {
        (covered.len() as f64 / total as f64) * 100.0
    } else {
        100.0
    };

    Ok(CoverageReport {
        covered,
        gaps,
        total,
        percent,
    })
}

/// Render coverage report as markdown.
pub fn render_markdown(report: &CoverageReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Test Coverage Report\n\n**{}/{} endpoints covered ({:.1}%)**\n\n",
        report.covered.len(),
        report.total,
        report.percent
    ));

    if !report.gaps.is_empty() {
        out.push_str("## Gaps (uncovered endpoints)\n\n");
        out.push_str("| Method | Path | Spec |\n|--------|------|------|\n");
        for ep in &report.gaps {
            out.push_str(&format!(
                "| {} | {} | {} |\n",
                ep.method, ep.path, ep.spec_file
            ));
        }
        out.push('\n');
    }

    if !report.covered.is_empty() {
        out.push_str("## Covered endpoints\n\n");
        out.push_str("| Method | Path | Spec |\n|--------|------|------|\n");
        for ep in &report.covered {
            out.push_str(&format!(
                "| {} | {} | {} |\n",
                ep.method, ep.path, ep.spec_file
            ));
        }
    }

    out
}

/// Render coverage report as JSON.
pub fn render_json(report: &CoverageReport) -> serde_json::Value {
    serde_json::json!({
        "total": report.total,
        "covered_count": report.covered.len(),
        "gap_count": report.gaps.len(),
        "percent": report.percent,
        "gaps": report.gaps.iter().map(|ep| serde_json::json!({
            "method": ep.method,
            "path": ep.path,
            "spec": ep.spec_file,
        })).collect::<Vec<_>>(),
        "covered": report.covered.iter().map(|ep| serde_json::json!({
            "method": ep.method,
            "path": ep.path,
            "spec": ep.spec_file,
        })).collect::<Vec<_>>(),
    })
}

fn load_existing_test_cases(project_root: &Path) -> Vec<TestCase> {
    let paths = match walk_tests::walk_tests_recursive(project_root, &["yaml", "yml"]) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let mut cases = Vec::new();
    for path in paths {
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        // Try parsing as a TestCase directly
        if let Ok(tc) = serde_yaml_ng::from_str::<TestCase>(&raw) {
            cases.push(tc);
            continue;
        }
        // Try parsing as a YamlSuite (user-defined test format) and
        // extract the name as a potential endpoint reference.
        if let Ok(suite) = serde_yaml_ng::from_str::<SuiteNameOnly>(&raw) {
            // Build a minimal TestCase from the suite name so the
            // endpoint extraction logic can match it.
            cases.push(TestCase {
                id: format!("file:{}", path.display()),
                name: suite.name,
                kind: TestKind::UserDefined,
                service: suite.service,
                tags: suite.tags,
                source: crate::spec::TestSource::Inline,
                spec: crate::spec::TestSpec::empty(),
            });
        }
    }
    cases
}

/// Minimal deserialization target: we only need the `name` field from
/// a YamlSuite to extract the endpoint pattern.
#[derive(serde::Deserialize)]
struct SuiteNameOnly {
    #[serde(default)]
    name: String,
    #[serde(default)]
    service: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

fn extract_endpoint_from_case(tc: &TestCase) -> Option<(String, String)> {
    // Test case names follow the pattern "METHOD /path" or are tagged
    // with endpoint info. Try to parse from name first.
    let name = &tc.name;
    let parts: Vec<&str> = name.splitn(2, ' ').collect();
    if parts.len() == 2 {
        let method = parts[0].to_lowercase();
        if ["get", "post", "put", "patch", "delete", "head", "options"].contains(&method.as_str()) {
            return Some((method, parts[1].to_string()));
        }
    }
    // Also try the "openapi METHOD /path" pattern used by discover
    if let Some(rest) = name.strip_prefix("openapi ") {
        let parts: Vec<&str> = rest.splitn(2, ' ').collect();
        if parts.len() == 2 {
            let method = parts[0].to_lowercase();
            if ["get", "post", "put", "patch", "delete", "head", "options"]
                .contains(&method.as_str())
            {
                return Some((method, parts[1].to_string()));
            }
        }
    }
    None
}

fn endpoint_from_discovered(d: &DiscoveredCase) -> Endpoint {
    let name = &d.case.name;
    // Discovered case names follow "openapi METHOD /path"
    let (method, path) = if let Some(rest) = name.strip_prefix("openapi ") {
        let parts: Vec<&str> = rest.splitn(2, ' ').collect();
        if parts.len() == 2 {
            (parts[0].to_uppercase(), parts[1].to_string())
        } else {
            ("GET".to_string(), name.clone())
        }
    } else {
        let parts: Vec<&str> = name.splitn(2, ' ').collect();
        if parts.len() == 2 {
            (parts[0].to_uppercase(), parts[1].to_string())
        } else {
            ("GET".to_string(), name.clone())
        }
    };
    Endpoint {
        method,
        path,
        spec_file: d.source_spec.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{TestSource, TestSpec};

    #[test]
    fn extract_endpoint_from_method_path_name() {
        let tc = TestCase {
            id: "test-1".into(),
            name: "GET /users".into(),
            kind: TestKind::UserDefined,
            service: Some("api".into()),
            tags: vec![],
            source: TestSource::Inline,
            spec: TestSpec::empty(),
        };
        let ep = extract_endpoint_from_case(&tc);
        assert_eq!(ep, Some(("get".to_string(), "/users".to_string())));
    }

    #[test]
    fn extract_endpoint_from_openapi_name() {
        let tc = TestCase {
            id: "test-2".into(),
            name: "openapi GET /health".into(),
            kind: TestKind::UserDefined,
            service: None,
            tags: vec![],
            source: TestSource::Inline,
            spec: TestSpec::empty(),
        };
        let ep = extract_endpoint_from_case(&tc);
        assert_eq!(ep, Some(("get".to_string(), "/health".to_string())));
    }

    #[test]
    fn extract_endpoint_returns_none_for_non_endpoint_names() {
        let tc = TestCase {
            id: "test-3".into(),
            name: "my integration test".into(),
            kind: TestKind::UserDefined,
            service: None,
            tags: vec![],
            source: TestSource::Inline,
            spec: TestSpec::empty(),
        };
        let ep = extract_endpoint_from_case(&tc);
        assert_eq!(ep, None);
    }

    #[test]
    fn coverage_empty_project() {
        let tmp = tempfile::tempdir().unwrap();
        let report = compute_coverage(tmp.path()).unwrap();
        assert_eq!(report.total, 0);
        assert_eq!(report.percent, 100.0);
        assert!(report.gaps.is_empty());
        assert!(report.covered.is_empty());
    }

    #[test]
    fn coverage_with_uncovered_endpoints() {
        let tmp = tempfile::tempdir().unwrap();
        // Create an OpenAPI spec with endpoints
        std::fs::write(
            tmp.path().join("openapi.yaml"),
            r#"openapi: 3.0.0
info: { title: Demo, version: 1.0.0 }
paths:
  /users:
    get:
      responses:
        '200':
          description: ok
  /health:
    get:
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();
        let report = compute_coverage(tmp.path()).unwrap();
        assert_eq!(report.total, 2);
        assert_eq!(report.gaps.len(), 2);
        assert_eq!(report.covered.len(), 0);
        assert!((report.percent - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn coverage_with_matching_test() {
        let tmp = tempfile::tempdir().unwrap();
        // Create an OpenAPI spec
        std::fs::write(
            tmp.path().join("openapi.yaml"),
            r#"openapi: 3.0.0
info: { title: Demo, version: 1.0.0 }
paths:
  /users:
    get:
      responses:
        '200':
          description: ok
"#,
        )
        .unwrap();
        // Create a matching test case
        let tests_dir = tmp.path().join(".coral/tests");
        std::fs::create_dir_all(&tests_dir).unwrap();
        std::fs::write(
            tests_dir.join("users.yaml"),
            r#"name: "openapi GET /users"
service: api
tags: [smoke]
"#,
        )
        .unwrap();
        let report = compute_coverage(tmp.path()).unwrap();
        assert_eq!(report.total, 1);
        assert_eq!(report.covered.len(), 1);
        assert_eq!(report.gaps.len(), 0);
        assert!((report.percent - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn render_markdown_shows_gaps_and_covered() {
        let report = CoverageReport {
            covered: vec![Endpoint {
                method: "GET".into(),
                path: "/health".into(),
                spec_file: "openapi.yaml".into(),
            }],
            gaps: vec![Endpoint {
                method: "POST".into(),
                path: "/users".into(),
                spec_file: "openapi.yaml".into(),
            }],
            total: 2,
            percent: 50.0,
        };
        let md = render_markdown(&report);
        assert!(md.contains("1/2 endpoints covered (50.0%)"));
        assert!(md.contains("POST"));
        assert!(md.contains("/users"));
        assert!(md.contains("GET"));
        assert!(md.contains("/health"));
    }

    #[test]
    fn render_json_structure() {
        let report = CoverageReport {
            covered: vec![],
            gaps: vec![Endpoint {
                method: "GET".into(),
                path: "/x".into(),
                spec_file: "s.yaml".into(),
            }],
            total: 1,
            percent: 0.0,
        };
        let json = render_json(&report);
        assert_eq!(json["total"], 1);
        assert_eq!(json["gap_count"], 1);
        assert_eq!(json["covered_count"], 0);
        assert_eq!(json["gaps"][0]["method"], "GET");
    }
}
