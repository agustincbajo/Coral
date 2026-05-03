//! Test schema — `TestKind`, `TestCase`, `TestSpec`.
//!
//! v0.18 wave 1 ships the type model + serde derive. The four MVP
//! runners (Healthcheck, UserDefined YAML, Hurl, Discovery) consume
//! these in wave 2.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 9 test kinds, mirroring the PRD §3.3 taxonomy. v0.18 ships
/// `Healthcheck` + `UserDefined`; the rest are reserved schema (so a
/// manifest using `kind = "contract"` doesn't break v0.18, it just
/// returns `Skip { reason = "v0.19 feature" }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestKind {
    Healthcheck,
    UserDefined,
    LlmGenerated,
    Contract,
    PropertyBased,
    Recorded,
    Event,
    Trace,
    E2eBrowser,
}

/// A single test case — the unit of execution. `spec` carries the
/// kind-specific payload (HTTP steps for `UserDefined`, target
/// service for `Healthcheck`, Schemathesis seed for `PropertyBased`,
/// …); the runner downcasts based on `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestCase {
    pub id: String,
    pub name: String,
    pub kind: TestKind,
    pub service: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub source: TestSource,
    pub spec: TestSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TestSource {
    #[default]
    Inline,
    File {
        path: PathBuf,
    },
    Discovered {
        from: String, // "openapi.yaml", "asyncapi.yaml", "proto://service.method"
    },
    Generated {
        runner: String,
        prompt_version: String,
        iter_count: u32,
        reviewed: bool,
    },
}

/// The kind-specific payload. v0.18 wave 1 keeps this opaque
/// (`serde_json::Value`) so we don't pre-commit to a final shape;
/// each runner v0.18 wave 2 will deserialize the value into its own
/// strongly-typed Spec struct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TestSpec(pub serde_json::Value);

impl TestSpec {
    pub fn empty() -> Self {
        Self(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_round_trips_via_yaml() {
        let yaml = "healthcheck\n";
        let parsed: TestKind = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(parsed, TestKind::Healthcheck);
    }

    #[test]
    fn test_case_round_trips_via_yaml() {
        let case = TestCase {
            id: "smoke-1".into(),
            name: "api smoke".into(),
            kind: TestKind::UserDefined,
            service: Some("api".into()),
            tags: vec!["smoke".into()],
            source: TestSource::Inline,
            spec: TestSpec::empty(),
        };
        let yaml = serde_yaml_ng::to_string(&case).unwrap();
        let parsed: TestCase = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(parsed, case);
    }

    #[test]
    fn test_source_default_is_inline() {
        let s = TestSource::default();
        assert_eq!(s, TestSource::Inline);
    }
}
