//! On-disk schema of the `[[environments]]` table in `coral.toml`.
//!
//! v0.16.0 declared `Project` without an `environments` field. v0.17
//! adds it as an optional table — single-repo and multi-repo projects
//! that don't need an environment keep working unchanged.
//!
//! Lives in `coral-env` (rather than `coral-core`) because it's the
//! data model the backends consume; nothing in the wiki layer needs
//! these types.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A single environment declared in `coral.toml` (e.g. `dev`, `ci`,
/// `staging`). v0.17 keeps the schema small; activation conditions and
/// `production = true` flags follow in v0.17.x as we wire them through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentSpec {
    pub name: String,
    pub backend: String,
    #[serde(default)]
    pub mode: EnvMode,
    #[serde(default = "default_compose_command")]
    pub compose_command: String,
    #[serde(default)]
    pub production: bool,
    pub env_file: Option<PathBuf>,
    pub services: BTreeMap<String, ServiceKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EnvMode {
    /// Coral generates the docker-compose.yml from `[environments.<env>]`.
    #[default]
    Managed,
    /// User brings their own compose file; Coral just invokes it.
    Adopt,
}

fn default_compose_command() -> String {
    "auto".to_string()
}

/// A service entry. v0.17 supports two kinds: a real container (with
/// build context or image) and a mock (placeholder for v0.18+; kept in
/// the schema so the manifest doesn't break when v0.18 lands).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServiceKind {
    /// A real container. One of `image` or `build` must be set.
    /// Boxed because `RealService` is much larger than `MockService`
    /// and we don't want every `ServiceKind` value to pay the size of
    /// the largest variant on the stack.
    Real(Box<RealService>),
    /// (v0.18+) A mock server (Mockoon / WireMock / Hoverfly).
    Mock(MockService),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RealService {
    /// Repo from `[[repos]]` whose checkout provides the build context.
    /// Mutually exclusive with `image`.
    pub repo: Option<String>,
    /// Pre-built image. Mutually exclusive with `repo`/`build`.
    pub image: Option<String>,
    /// Build sub-table (Garden-style separation, future-proof).
    #[serde(default)]
    pub build: Option<BuildSpec>,
    #[serde(default)]
    pub ports: Vec<u16>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub healthcheck: Option<Healthcheck>,
    /// Watch (compose 2.22+ `develop.watch`).
    #[serde(default)]
    pub watch: Option<WatchSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildSpec {
    /// Build context relative to the repo's checkout root.
    #[serde(default = "default_dot")]
    pub context: PathBuf,
    pub dockerfile: Option<PathBuf>,
    /// Multi-stage target.
    pub target: Option<String>,
    #[serde(default)]
    pub cache_from: Vec<String>,
    pub cache_to: Option<String>,
    #[serde(default)]
    pub args: BTreeMap<String, String>,
}

fn default_dot() -> PathBuf {
    PathBuf::from(".")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchSpec {
    #[serde(default)]
    pub sync: Vec<SyncRule>,
    #[serde(default)]
    pub rebuild: Vec<String>,
    #[serde(default)]
    pub restart: Vec<String>,
    #[serde(default)]
    pub initial_sync: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncRule {
    pub path: PathBuf,
    pub target: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MockService {
    pub tool: String, // "mockoon" | "wiremock" | "hoverfly"
    pub spec: Option<PathBuf>,
    pub mappings_dir: Option<PathBuf>,
    pub mode: Option<String>, // hoverfly: capture | simulate | spy
    pub recording: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Healthcheck {
    #[serde(flatten)]
    pub kind: HealthcheckKind,
    #[serde(default)]
    pub timing: HealthcheckTiming,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HealthcheckKind {
    Http {
        path: String,
        #[serde(default = "default_200")]
        expect_status: u16,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    Tcp {
        port: u16,
    },
    Exec {
        cmd: Vec<String>,
    },
    Grpc {
        port: u16,
        service: Option<String>,
    },
}

fn default_200() -> u16 {
    200
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthcheckTiming {
    #[serde(default = "default_interval_s")]
    pub interval_s: u32,
    #[serde(default = "default_timeout_s")]
    pub timeout_s: u32,
    #[serde(default = "default_retries")]
    pub retries: u32,
    #[serde(default = "default_start_period_s")]
    pub start_period_s: u32,
    #[serde(default)]
    pub start_interval_s: Option<u32>,
    #[serde(default = "default_consecutive_failures")]
    pub consecutive_failures: u32,
}

impl Default for HealthcheckTiming {
    fn default() -> Self {
        Self {
            interval_s: default_interval_s(),
            timeout_s: default_timeout_s(),
            retries: default_retries(),
            start_period_s: default_start_period_s(),
            start_interval_s: None,
            consecutive_failures: default_consecutive_failures(),
        }
    }
}

fn default_interval_s() -> u32 {
    5
}
fn default_timeout_s() -> u32 {
    3
}
fn default_retries() -> u32 {
    5
}
fn default_start_period_s() -> u32 {
    30
}
fn default_consecutive_failures() -> u32 {
    3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_defaults_are_sane() {
        let t = HealthcheckTiming::default();
        assert_eq!(t.interval_s, 5);
        assert_eq!(t.consecutive_failures, 3);
    }

    #[test]
    fn http_kind_round_trips() {
        let raw = r#"{ "kind": "http", "path": "/health" }"#;
        let parsed: HealthcheckKind = serde_json::from_str(raw).unwrap();
        match parsed {
            HealthcheckKind::Http {
                path,
                expect_status,
                ..
            } => {
                assert_eq!(path, "/health");
                assert_eq!(expect_status, 200); // default
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn grpc_kind_parses() {
        let raw = r#"{ "kind": "grpc", "port": 50051, "service": "health.v1" }"#;
        let parsed: HealthcheckKind = serde_json::from_str(raw).unwrap();
        match parsed {
            HealthcheckKind::Grpc { port, service } => {
                assert_eq!(port, 50051);
                assert_eq!(service.as_deref(), Some("health.v1"));
            }
            other => panic!("expected Grpc, got {other:?}"),
        }
    }

    #[test]
    fn watch_spec_defaults_initial_sync_to_false() {
        let raw = r#"{ "sync": [], "rebuild": [], "restart": [] }"#;
        let parsed: WatchSpec = serde_json::from_str(raw).unwrap();
        assert!(!parsed.initial_sync);
    }
}
