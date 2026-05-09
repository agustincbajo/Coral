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
    /// v0.23.1: pre-canned monitors runnable via `coral monitor up`.
    /// Each monitor pairs a TestCase filter (tag / kind / services) with a
    /// cron-like interval. Empty when the environment doesn't declare any.
    /// `skip_serializing_if` keeps v0.23.0 manifests byte-identical when
    /// no `[[environments.<env>.monitors]]` blocks are present — see
    /// `monitors_absent_round_trips_unchanged`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub monitors: Vec<MonitorSpec>,
    /// v0.23.2: recorded-test replay configuration. When present,
    /// `coral test --kind recorded` replays Keploy-captured exchanges
    /// stored under `.coral/tests/recorded/<service>/*.yaml` against
    /// the live env. The `ignore_response_fields` list is recursively
    /// stripped from response bodies before deep-equal comparison so
    /// dynamic fields (`id`, `timestamp`) don't false-positive.
    /// `skip_serializing_if` keeps v0.23.1 manifests byte-identical
    /// when no `[environments.<env>.recorded]` block is present —
    /// pinned by `recorded_config_absent_round_trips_unchanged`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded: Option<RecordedConfig>,
}

/// v0.23.1: a single monitor entry under `[[environments.<env>.monitors]]`.
///
/// A monitor is a *named, scheduled invocation of `coral test` against
/// a long-lived environment*. Filters mirror the `coral test --tag` /
/// `--kind` / `--service` shape exactly so the same `coral.toml` lines
/// can be lifted into a monitor without re-authoring the filter spec.
///
/// `kind` is stored as `Option<String>` rather than `Option<TestKind>`
/// because `coral-env` is upstream of `coral-test` (`TestKind` lives
/// there). The CLI's `monitor up` resolves this to a `TestKind` at
/// dispatch time and bails with an actionable error on mismatch — the
/// validation surfaces in `parse_all` via `EnvironmentSpec::validate`.
///
/// `MonitorSpec` is **frozen** for v0.23.1: any new field MUST land
/// with `#[serde(default, skip_serializing_if = ...)]` so existing
/// manifests round-trip unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorSpec {
    pub name: String,
    /// Optional tag filter (e.g. `"smoke"`). When absent, the monitor
    /// runs every TestCase the env exposes (subject to other filters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Optional kind filter (`"healthcheck"`, `"user_defined"`,
    /// `"smoke"`). Stored as a string here — the CLI parses it into
    /// `coral_test::TestKind` at dispatch time. See
    /// `EnvironmentSpec::validate` for the parse-time check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Optional service-name filter (repeatable in TOML via
    /// `services = ["api", "db"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<String>,
    /// How often the monitor fires. v0.23.1 ships seconds resolution;
    /// minute / hour cron is deferred to v0.23.x+ pending demand.
    pub interval_seconds: u64,
    /// What to do when an iteration fails. Default: `Log` — record the
    /// pass/fail tally to JSONL and continue on the next tick.
    #[serde(default)]
    pub on_failure: OnFailure,
}

/// v0.23.1: how to react to a failed monitor iteration. `Alert` parses
/// (so the manifest is forward-compatible) but errors at runtime in
/// v0.23.1 — the alert wiring (PagerDuty / OpsGenie webhooks) lands in
/// v0.24+.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum OnFailure {
    /// Append the run to JSONL and continue. The default.
    #[default]
    Log,
    /// Append the run to JSONL and exit non-zero immediately.
    FailFast,
    /// Reserved. Currently errors at runtime (see `monitor up`).
    Alert,
}

/// v0.23.2: recorded-test replay configuration. Lives under
/// `[environments.<env>.recorded]`:
///
/// ```toml
/// [environments.dev.recorded]
/// ignore_response_fields = ["id", "timestamp", "created_at", "request_id"]
/// ```
///
/// The list is applied recursively when comparing the captured response
/// body against the live one — a key named `id` at any depth in the
/// response JSON is stripped from BOTH sides before deep-equal compare.
/// This keeps replay tests stable across timestamps, UUIDs, and
/// auto-incrementing IDs.
///
/// **Frozen for v0.23.2:** any new field MUST land with
/// `#[serde(default, skip_serializing_if = ...)]` so existing manifests
/// round-trip unchanged on a future binary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RecordedConfig {
    /// Response-body field names to strip recursively before comparing
    /// captured vs. live responses. Empty = no fields ignored (every
    /// JSON field is structurally compared).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore_response_fields: Vec<String>,
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
        // ---- v0.23.1 monitors ----
        // Cheap pre-flight checks against the manifest shape. The CLI
        // re-validates `kind` against `coral_test::TestKind` at
        // dispatch time (the string parse lives in the consumer crate).
        for m in &self.monitors {
            if m.name.is_empty() {
                return Err(format!(
                    "environment '{}' declares a monitor with an empty name; every \
                     `[[environments.{}.monitors]]` block needs a `name = \"...\"`",
                    self.name, self.name
                ));
            }
            if m.interval_seconds == 0 {
                return Err(format!(
                    "monitor '{}' in environment '{}' has interval_seconds = 0; pick a positive value",
                    m.name, self.name
                ));
            }
            if let Some(k) = &m.kind {
                let known = matches!(
                    k.as_str(),
                    "healthcheck" | "user_defined" | "smoke" | "contract" | "property_based"
                );
                if !known {
                    return Err(format!(
                        "monitor '{}' in environment '{}' has unknown kind '{}'; \
                         valid: healthcheck, user_defined, smoke",
                        m.name, self.name, k
                    ));
                }
            }
        }
        // Names must be unique within an env so JSONL paths
        // (`<env>-<monitor>.jsonl`) don't collide.
        {
            let mut seen = std::collections::BTreeSet::new();
            for m in &self.monitors {
                if !seen.insert(m.name.as_str()) {
                    return Err(format!(
                        "environment '{}' declares two monitors named '{}'; \
                         names must be unique within an env",
                        self.name, m.name
                    ));
                }
            }
        }

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
            monitors: Vec::new(),
            recorded: None,
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
            monitors: Vec::new(),
            recorded: None,
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
            monitors: Vec::new(),
            recorded: None,
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
            monitors: Vec::new(),
            recorded: None,
        };
        let err = spec.validate().expect_err("must reject");
        assert!(err.contains("unknown_key"), "wrong msg: {err}");
    }

    // ---- v0.23.1: monitors ----

    /// **BC golden — T1.** A v0.23.0 manifest without
    /// `[[environments.<env>.monitors]]` MUST round-trip byte-identically
    /// on a v0.23.1 binary. Pre-v0.23.1 manifests still work because
    /// `monitors` is `skip_serializing_if = "Vec::is_empty"`.
    #[test]
    fn monitors_absent_round_trips_unchanged() {
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
            monitors: Vec::new(),
            recorded: None,
        };
        let serialized = toml::to_string(&spec).expect("serialize");
        // The serialized form must NOT mention `monitors` because it's
        // skip-on-default.
        assert!(
            !serialized.contains("monitors"),
            "v0.23.0 manifest serialized with monitors noise: {serialized}"
        );
        let reparsed: EnvironmentSpec = toml::from_str(&serialized).expect("reparse");
        assert_eq!(reparsed, spec, "monitors-absent spec did not round-trip");
        spec.validate().expect("validate clean spec");
    }

    /// Parse a full `[[environments.<env>.monitors]]` block: tag, kind,
    /// services, interval, on_failure all populated.
    #[test]
    fn monitors_full_section_parses() {
        let toml_src = r#"
name = "staging"
backend = "compose"
[services.api]
kind = "real"
image = "api:dev"

[[monitors]]
name = "smoke-loop"
tag = "smoke"
kind = "user_defined"
services = ["api"]
interval_seconds = 60
on_failure = "log"

[[monitors]]
name = "fail-fast-canary"
interval_seconds = 30
on_failure = "fail-fast"
"#;
        let spec: EnvironmentSpec = toml::from_str(toml_src).expect("parse");
        assert_eq!(spec.monitors.len(), 2);
        let smoke = &spec.monitors[0];
        assert_eq!(smoke.name, "smoke-loop");
        assert_eq!(smoke.tag.as_deref(), Some("smoke"));
        assert_eq!(smoke.kind.as_deref(), Some("user_defined"));
        assert_eq!(smoke.services, vec!["api".to_string()]);
        assert_eq!(smoke.interval_seconds, 60);
        assert_eq!(smoke.on_failure, OnFailure::Log);

        let canary = &spec.monitors[1];
        assert_eq!(canary.name, "fail-fast-canary");
        assert_eq!(canary.tag, None);
        assert_eq!(canary.kind, None);
        assert!(canary.services.is_empty());
        assert_eq!(canary.interval_seconds, 30);
        assert_eq!(canary.on_failure, OnFailure::FailFast);

        spec.validate().expect("validate full spec");
    }

    /// `on_failure = "alert"` parses (forward-compat) but does NOT error
    /// at validate — the runtime "reserved for v0.24+" check lives in
    /// the monitor up handler, not here.
    #[test]
    fn monitors_alert_on_failure_parses_validate_passes() {
        let toml_src = r#"
name = "dev"
backend = "compose"
[services.api]
kind = "real"
image = "api:dev"

[[monitors]]
name = "alert-canary"
interval_seconds = 60
on_failure = "alert"
"#;
        let spec: EnvironmentSpec = toml::from_str(toml_src).expect("parse");
        assert_eq!(spec.monitors[0].on_failure, OnFailure::Alert);
        spec.validate().expect("validate accepts alert");
    }

    #[test]
    fn monitor_with_zero_interval_rejected() {
        let mut spec = base_spec_for_monitors();
        spec.monitors.push(MonitorSpec {
            name: "bad".into(),
            tag: None,
            kind: None,
            services: vec![],
            interval_seconds: 0,
            on_failure: OnFailure::Log,
        });
        let err = spec.validate().expect_err("must reject");
        assert!(err.contains("interval_seconds = 0"), "wrong msg: {err}");
    }

    #[test]
    fn monitor_with_unknown_kind_rejected() {
        let mut spec = base_spec_for_monitors();
        spec.monitors.push(MonitorSpec {
            name: "weird".into(),
            tag: None,
            kind: Some("flux-capacitor".into()),
            services: vec![],
            interval_seconds: 30,
            on_failure: OnFailure::Log,
        });
        let err = spec.validate().expect_err("must reject");
        assert!(err.contains("unknown kind"), "wrong msg: {err}");
    }

    #[test]
    fn monitor_duplicate_names_rejected() {
        let mut spec = base_spec_for_monitors();
        spec.monitors.push(MonitorSpec {
            name: "dup".into(),
            tag: None,
            kind: None,
            services: vec![],
            interval_seconds: 30,
            on_failure: OnFailure::Log,
        });
        spec.monitors.push(MonitorSpec {
            name: "dup".into(),
            tag: None,
            kind: None,
            services: vec![],
            interval_seconds: 60,
            on_failure: OnFailure::Log,
        });
        let err = spec.validate().expect_err("must reject");
        assert!(err.contains("two monitors"), "wrong msg: {err}");
    }

    fn base_spec_for_monitors() -> EnvironmentSpec {
        EnvironmentSpec {
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
            chaos_scenarios: Vec::new(),
            monitors: Vec::new(),
            recorded: None,
        }
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
            monitors: Vec::new(),
            recorded: None,
        };
        let err = spec.validate().expect_err("must reject");
        assert!(err.contains("reserved"), "wrong msg: {err}");
    }

    // ---- v0.23.2: recorded ----

    /// **BC golden — T1 for v0.23.2.** A v0.23.1-shaped manifest without
    /// `[environments.<env>.recorded]` MUST round-trip byte-identically
    /// on the v0.23.2 binary. The `recorded` field is `Option<_>` and
    /// `skip_serializing_if = "Option::is_none"` so manifests
    /// pre-recorded never carry a `recorded` line.
    #[test]
    fn recorded_config_absent_round_trips_unchanged() {
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
            monitors: Vec::new(),
            recorded: None,
        };
        let serialized = toml::to_string(&spec).expect("serialize");
        // The serialized form must NOT mention `recorded` because it's
        // skip-on-None.
        assert!(
            !serialized.contains("recorded"),
            "v0.23.1 manifest serialized with recorded noise: {serialized}"
        );
        let reparsed: EnvironmentSpec = toml::from_str(&serialized).expect("reparse");
        assert_eq!(reparsed, spec, "recorded-absent spec did not round-trip");
        spec.validate().expect("validate clean spec");
    }

    /// Parse a full `[environments.<env>.recorded]` block with
    /// `ignore_response_fields` populated. Acceptance criterion #7.
    #[test]
    fn recorded_config_with_ignore_fields_parses() {
        let toml_src = r#"
name = "dev"
backend = "compose"
[services.api]
kind = "real"
image = "api:dev"

[recorded]
ignore_response_fields = ["id", "timestamp", "created_at", "request_id"]
"#;
        let spec: EnvironmentSpec = toml::from_str(toml_src).expect("parse");
        let recorded = spec.recorded.as_ref().expect("recorded block parsed");
        assert_eq!(
            recorded.ignore_response_fields,
            vec![
                "id".to_string(),
                "timestamp".to_string(),
                "created_at".to_string(),
                "request_id".to_string()
            ]
        );
        spec.validate().expect("recorded config validates");
    }
}
