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
///
/// v0.23.0 adds the optional `chaos` and `chaos_scenarios` fields so a
/// project can declare a Toxiproxy sidecar plus pre-canned scenarios
/// (`coral chaos run high-latency`). Both fields are
/// `skip_serializing_if`'d so a manifest without `[environments.<env>.chaos]`
/// round-trips byte-identically to v0.22.6 — see
/// `chaos_config_absent_round_trips_unchanged`.
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
    /// Optional chaos-engineering sidecar. v0.23.0 ships Toxiproxy only
    /// (Pumba deferred per D1 in the orchestrator's spec).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chaos: Option<ChaosConfig>,
    /// Pre-canned chaos scenarios runnable via `coral chaos run <name>`.
    /// Empty when the environment doesn't declare any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chaos_scenarios: Vec<ChaosScenario>,
}

/// Chaos-engineering sidecar configuration. v0.23.0 only knows
/// Toxiproxy; the `backend` field is a kebab-case enum so a future
/// Pumba/Litmus addition is a strictly additive change.
///
/// Backend is `#[non_exhaustive]` so adding a variant in v0.24+ doesn't
/// require an exhaustive `match` in downstream callers — they just hit
/// the catch-all and get a friendly "unknown backend" error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChaosConfig {
    pub backend: ChaosBackend,
    #[serde(default = "default_chaos_image")]
    pub image: String,
    #[serde(default = "default_chaos_listen_port")]
    pub listen_port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ChaosBackend {
    Toxiproxy,
}

/// A named chaos scenario — a preset list of toxic + service +
/// attributes that `coral chaos run <name>` dispatches to the
/// `inject` path. Per-attribute validation is `ToxicKind`-driven; see
/// `EnvironmentSpec::validate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChaosScenario {
    pub name: String,
    pub toxic: ToxicKind,
    pub service: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, toml::Value>,
}

/// The five Toxiproxy toxic types we expose in v0.23.0. Direct mapping
/// to the wire-level `type` field in the toxiproxy admin API so
/// `serde_json::to_string(toxic)` produces a payload-ready value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToxicKind {
    Latency,
    Bandwidth,
    SlowClose,
    Timeout,
    Slicer,
}

impl ToxicKind {
    /// The exact `type` string the Toxiproxy admin API expects.
    pub fn as_api_str(&self) -> &'static str {
        match self {
            ToxicKind::Latency => "latency",
            ToxicKind::Bandwidth => "bandwidth",
            ToxicKind::SlowClose => "slow_close",
            ToxicKind::Timeout => "timeout",
            ToxicKind::Slicer => "slicer",
        }
    }

    /// Required attribute keys for this toxic. Used by the validator.
    pub fn required_attributes(&self) -> &'static [&'static str] {
        match self {
            ToxicKind::Latency => &["latency"],
            ToxicKind::Bandwidth => &["rate"],
            ToxicKind::SlowClose => &["delay"],
            ToxicKind::Timeout => &[],
            ToxicKind::Slicer => &[],
        }
    }

    /// Optional attribute keys. Combined with `required_attributes`
    /// to form the allow-list (anything else fails validation with a
    /// "unknown attribute" error).
    pub fn optional_attributes(&self) -> &'static [&'static str] {
        match self {
            ToxicKind::Latency => &["jitter"],
            ToxicKind::Bandwidth => &[],
            ToxicKind::SlowClose => &[],
            ToxicKind::Timeout => &["timeout"],
            ToxicKind::Slicer => &["average_size", "size_variation", "delay"],
        }
    }
}

fn default_chaos_image() -> String {
    "ghcr.io/shopify/toxiproxy:2.7.0".to_string()
}

fn default_chaos_listen_port() -> u16 {
    8474
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

/// Reserved service name. v0.23.0+: when `[environments.<env>.chaos]`
/// is set we synthesize a sidecar called `toxiproxy`; if a user already
/// declares a service with the same name, validation fails so the
/// generated YAML doesn't silently overwrite the user's definition.
pub(crate) const TOXIPROXY_SIDECAR_NAME: &str = "toxiproxy";

impl EnvironmentSpec {
    /// Validate v0.23.0 chaos invariants beyond what `serde` already
    /// catches. Called from the CLI's `resolve_env` path, the import
    /// path, and the BC-pin tests.
    ///
    /// Rules:
    ///
    /// 1. `chaos_scenarios` non-empty implies `chaos = Some(_)`.
    /// 2. Every `chaos_scenarios[*].service` must exist in `services`.
    /// 3. `attributes` keys must match the toxic kind's allow-list:
    ///    required keys present, no unknown keys.
    /// 4. `services["toxiproxy"]` is reserved when `chaos.is_some()`.
    pub fn validate(&self) -> Result<(), String> {
        if !self.chaos_scenarios.is_empty() && self.chaos.is_none() {
            return Err(format!(
                "environment '{}' declares chaos_scenarios but no [environments.{}.chaos] block; \
                 add `[environments.{}.chaos] backend = \"toxiproxy\"`",
                self.name, self.name, self.name
            ));
        }
        if self.chaos.is_some() && self.services.contains_key(TOXIPROXY_SIDECAR_NAME) {
            return Err(format!(
                "environment '{}' declares a service named '{}', which is reserved \
                 for the chaos sidecar (v0.23.0+); rename the user service",
                self.name, TOXIPROXY_SIDECAR_NAME
            ));
        }
        for scenario in &self.chaos_scenarios {
            if !self.services.contains_key(&scenario.service) {
                return Err(format!(
                    "chaos_scenario '{}' targets unknown service '{}' in environment '{}'",
                    scenario.name, scenario.service, self.name
                ));
            }
            let required = scenario.toxic.required_attributes();
            let optional = scenario.toxic.optional_attributes();
            for key in required {
                if !scenario.attributes.contains_key(*key) {
                    return Err(format!(
                        "chaos_scenario '{}' (toxic '{}') missing required attribute '{}'",
                        scenario.name,
                        scenario.toxic.as_api_str(),
                        key
                    ));
                }
            }
            for key in scenario.attributes.keys() {
                let known = required.contains(&key.as_str()) || optional.contains(&key.as_str());
                if !known {
                    return Err(format!(
                        "chaos_scenario '{}' (toxic '{}') has unknown attribute '{}'; \
                         valid: {}",
                        scenario.name,
                        scenario.toxic.as_api_str(),
                        key,
                        required
                            .iter()
                            .chain(optional.iter())
                            .copied()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }
        Ok(())
    }
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

    /// v0.21.2: serde must require both `path` and `target` on
    /// `SyncRule`. Pre-fix, a missing `target` would deserialize as
    /// the empty path and the renderer would emit `target: ""` —
    /// which `compose watch` rejects at runtime with an opaque
    /// error. Pin the round-trip so a future `#[serde(default)]`
    /// can't silently weaken the contract.
    #[test]
    fn sync_rule_requires_both_path_and_target() {
        let missing_target = r#"{ "path": "./src" }"#;
        let parsed: Result<SyncRule, _> = serde_json::from_str(missing_target);
        assert!(
            parsed.is_err(),
            "SyncRule without `target` must fail to deserialize"
        );
        let missing_path = r#"{ "target": "/app/src" }"#;
        let parsed: Result<SyncRule, _> = serde_json::from_str(missing_path);
        assert!(
            parsed.is_err(),
            "SyncRule without `path` must fail to deserialize"
        );
        // Sanity: both present succeeds.
        let ok = r#"{ "path": "./src", "target": "/app/src" }"#;
        let parsed: SyncRule = serde_json::from_str(ok).expect("must deserialize");
        assert_eq!(parsed.path, std::path::PathBuf::from("./src"));
        assert_eq!(parsed.target, std::path::PathBuf::from("/app/src"));
    }

    // ---- v0.23.0: chaos config ----

    /// **BC golden** — a manifest without `[environments.<env>.chaos]`
    /// (or `[[environments.<env>.chaos_scenarios]]`) MUST round-trip
    /// to byte-identical TOML. Pre-v0.23.0 manifests still work
    /// because both fields are `skip_serializing_if`.
    #[test]
    fn chaos_config_absent_round_trips_unchanged() {
        // Realistic v0.22.6-shape spec: backend, services, env_file
        // — nothing chaos-related.
        let spec = EnvironmentSpec {
            name: "dev".into(),
            backend: "compose".into(),
            mode: EnvMode::Managed,
            compose_command: "auto".into(),
            production: false,
            env_file: None,
            services: BTreeMap::from([(
                "api".into(),
                ServiceKind::Real(Box::new(RealService {
                    repo: None,
                    image: Some("api:dev".into()),
                    build: None,
                    ports: vec![3000],
                    env: BTreeMap::new(),
                    depends_on: vec![],
                    healthcheck: None,
                    watch: None,
                })),
            )]),
            chaos: None,
            chaos_scenarios: Vec::new(),
        };
        let serialized = toml::to_string(&spec).expect("serialize");
        // The serialized form must mention NEITHER `chaos` nor
        // `chaos_scenarios` because both are skip-on-default.
        assert!(
            !serialized.contains("chaos"),
            "v0.22.6 manifest serialized with chaos noise: {serialized}"
        );
        // Round-trip back through serde must equal the original spec.
        let reparsed: EnvironmentSpec = toml::from_str(&serialized).expect("reparse");
        assert_eq!(reparsed, spec, "chaos-absent spec did not round-trip");
        // Sanity: validate() must accept a chaos-absent spec.
        spec.validate().expect("validate clean spec");
    }

    #[test]
    fn chaos_config_with_toxiproxy_parses_full_section() {
        let toml_src = r#"
name = "dev"
backend = "compose"
[services.api]
kind = "real"
image = "api:dev"

[chaos]
backend = "toxiproxy"

[[chaos_scenarios]]
name = "high-latency"
toxic = "latency"
service = "api"
attributes = { latency = 500, jitter = 50 }
"#;
        let spec: EnvironmentSpec = toml::from_str(toml_src).expect("parse");
        let chaos = spec.chaos.as_ref().expect("chaos block parsed");
        assert_eq!(chaos.backend, ChaosBackend::Toxiproxy);
        // Default image + listen port apply when omitted.
        assert_eq!(chaos.image, "ghcr.io/shopify/toxiproxy:2.7.0");
        assert_eq!(chaos.listen_port, 8474);

        assert_eq!(spec.chaos_scenarios.len(), 1);
        let scenario = &spec.chaos_scenarios[0];
        assert_eq!(scenario.name, "high-latency");
        assert_eq!(scenario.toxic, ToxicKind::Latency);
        assert_eq!(scenario.service, "api");
        assert_eq!(
            scenario
                .attributes
                .get("latency")
                .and_then(|v| v.as_integer()),
            Some(500)
        );
        assert_eq!(
            scenario
                .attributes
                .get("jitter")
                .and_then(|v| v.as_integer()),
            Some(50)
        );
        spec.validate().expect("scenario validates");
    }

    #[test]
    fn chaos_unknown_backend_string_rejected_by_serde() {
        let toml_src = r#"
name = "dev"
backend = "compose"
[services.api]
kind = "real"
image = "api:dev"

[chaos]
backend = "pumba"
"#;
        let result: Result<EnvironmentSpec, _> = toml::from_str(toml_src);
        assert!(
            result.is_err(),
            "non-toxiproxy backend must be rejected at parse time"
        );
    }

    #[test]
    fn chaos_scenarios_without_block_fails_validate() {
        let mut spec = EnvironmentSpec {
            name: "dev".into(),
            backend: "compose".into(),
            mode: EnvMode::Managed,
            compose_command: "auto".into(),
            production: false,
            env_file: None,
            services: BTreeMap::from([(
                "api".into(),
                ServiceKind::Real(Box::new(RealService {
                    repo: None,
                    image: Some("api:dev".into()),
                    build: None,
                    ports: vec![],
                    env: BTreeMap::new(),
                    depends_on: vec![],
                    healthcheck: None,
                    watch: None,
                })),
            )]),
            chaos: None,
            chaos_scenarios: vec![ChaosScenario {
                name: "x".into(),
                toxic: ToxicKind::Latency,
                service: "api".into(),
                attributes: BTreeMap::from([("latency".into(), toml::Value::Integer(100))]),
            }],
        };
        let err = spec.validate().expect_err("must reject");
        assert!(err.contains("chaos_scenarios"), "wrong msg: {err}");
        // Adding the chaos block fixes it.
        spec.chaos = Some(ChaosConfig {
            backend: ChaosBackend::Toxiproxy,
            image: default_chaos_image(),
            listen_port: 8474,
        });
        spec.validate().expect("now valid");
    }

    #[test]
    fn chaos_scenario_targeting_unknown_service_rejected() {
        let spec = EnvironmentSpec {
            name: "dev".into(),
            backend: "compose".into(),
            mode: EnvMode::Managed,
            compose_command: "auto".into(),
            production: false,
            env_file: None,
            services: BTreeMap::from([(
                "api".into(),
                ServiceKind::Real(Box::new(RealService {
                    repo: None,
                    image: Some("api:dev".into()),
                    build: None,
                    ports: vec![],
                    env: BTreeMap::new(),
                    depends_on: vec![],
                    healthcheck: None,
                    watch: None,
                })),
            )]),
            chaos: Some(ChaosConfig {
                backend: ChaosBackend::Toxiproxy,
                image: default_chaos_image(),
                listen_port: 8474,
            }),
            chaos_scenarios: vec![ChaosScenario {
                name: "stale".into(),
                toxic: ToxicKind::Latency,
                service: "ghost".into(),
                attributes: BTreeMap::from([("latency".into(), toml::Value::Integer(100))]),
            }],
        };
        let err = spec.validate().expect_err("must reject");
        assert!(err.contains("ghost"), "wrong msg: {err}");
    }

    #[test]
    fn chaos_scenario_unknown_attribute_rejected() {
        let spec = EnvironmentSpec {
            name: "dev".into(),
            backend: "compose".into(),
            mode: EnvMode::Managed,
            compose_command: "auto".into(),
            production: false,
            env_file: None,
            services: BTreeMap::from([(
                "api".into(),
                ServiceKind::Real(Box::new(RealService {
                    repo: None,
                    image: Some("api:dev".into()),
                    build: None,
                    ports: vec![],
                    env: BTreeMap::new(),
                    depends_on: vec![],
                    healthcheck: None,
                    watch: None,
                })),
            )]),
            chaos: Some(ChaosConfig {
                backend: ChaosBackend::Toxiproxy,
                image: default_chaos_image(),
                listen_port: 8474,
            }),
            chaos_scenarios: vec![ChaosScenario {
                name: "wat".into(),
                toxic: ToxicKind::Latency,
                service: "api".into(),
                attributes: BTreeMap::from([
                    ("latency".into(), toml::Value::Integer(100)),
                    ("unknown_key".into(), toml::Value::Integer(0)),
                ]),
            }],
        };
        let err = spec.validate().expect_err("must reject");
        assert!(err.contains("unknown_key"), "wrong msg: {err}");
    }

    #[test]
    fn toxiproxy_service_name_reserved_when_chaos_on() {
        let spec = EnvironmentSpec {
            name: "dev".into(),
            backend: "compose".into(),
            mode: EnvMode::Managed,
            compose_command: "auto".into(),
            production: false,
            env_file: None,
            services: BTreeMap::from([(
                // User tried to declare a service called toxiproxy —
                // would silently collide with the v0.23.0 sidecar.
                "toxiproxy".into(),
                ServiceKind::Real(Box::new(RealService {
                    repo: None,
                    image: Some("user/their-toxiproxy:latest".into()),
                    build: None,
                    ports: vec![],
                    env: BTreeMap::new(),
                    depends_on: vec![],
                    healthcheck: None,
                    watch: None,
                })),
            )]),
            chaos: Some(ChaosConfig {
                backend: ChaosBackend::Toxiproxy,
                image: default_chaos_image(),
                listen_port: 8474,
            }),
            chaos_scenarios: vec![],
        };
        let err = spec.validate().expect_err("must reject");
        assert!(err.contains("reserved"), "wrong msg: {err}");
    }
}
