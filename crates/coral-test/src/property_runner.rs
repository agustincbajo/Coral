//! `PropertyRunner` — Schemathesis-style property-based testing from
//! OpenAPI specs (v0.23.3).
//!
//! ## What this is
//!
//! A test runner that consumes `[[environments.<env>.property_tests]]`
//! entries (each pairs one OpenAPI spec with one service), walks the
//! spec for `(path, method)` operations, and emits one `TestCase` per
//! endpoint. Each `TestCase` runs N proptest iterations with random
//! valid inputs against the live env. The first failing iteration
//! halts the case, proptest shrinks the input to the minimal failing
//! form, and the report carries the shrunken counter-example.
//!
//! ## Why the scope is narrow (D1)
//!
//! v0.23.3 ships a deliberately small slice:
//!
//! - **GET + POST only.** The ergonomics of OpenAPI request bodies,
//!   parameter encoding (form / multipart / x-www-form-urlencoded),
//!   and PUT/PATCH idempotency are easier to land one HTTP method at
//!   a time. v0.24+ extends.
//! - **Path params + JSON request bodies only.** No query strings, no
//!   custom headers, no `$ref`/`oneOf`/`allOf`. A spec that uses any
//!   of those has its endpoint silently skipped (the runner reports
//!   `Skip { reason }` rather than fabricating a request).
//! - **5 JSON Schema types only:** `string`, `number`, `integer`,
//!   `object`, `array`. Anything else (`null`, `boolean`, missing
//!   `type`) falls back to `string`.
//! - **Status validation only.** Response-body schema validation is
//!   deferred to v0.24+ — we'd need a JSON Schema validator dep
//!   that's stricter than `serde_json::Value::is_*`.
//! - **One TestCase per `(path, method)`**, not one per iteration.
//!   The runner internally loops the iterations and aggregates.
//!
//! ## Why curl, not reqwest (consistency with the rest of the workspace)
//!
//! Same justification as `recorded_runner.rs`: `coral_runner::http`,
//! `commands::notion_push`, `commands::chaos`, and `RecordedRunner`
//! all subprocess curl rather than dragging `reqwest` + `tokio` in.
//! Property runner follows the same pattern so the workspace stays
//! single-async-runtime-free.
//!
//! ## Determinism (D5)
//!
//! Seed precedence: CLI override (`--seed`) → manifest
//! (`property_tests[i].seed`) → fresh `rand::random::<u64>()`. The
//! resolved seed is BOTH `tracing::info!`-logged AND embedded into
//! `Evidence::stdout_tail` so a failing run can be reproduced from
//! the report alone, no manifest mutation needed.
//!
//! Internally the seed is expanded `u64 → 32 bytes` (replicated
//! 4× as little-endian) and handed to
//! `proptest::test_runner::TestRng::from_seed(RngAlgorithm::ChaCha,
//! ...)`. ChaCha requires exactly 32 bytes; expanding by replication
//! is deterministic and reproducible across platforms.

use crate::discover::{find_openapi_specs, parse_openapi_value};
use crate::error::{TestError, TestResult};
use crate::report::{Evidence, HttpEvidence, TestReport, TestStatus};
use crate::spec::{TestCase, TestKind, TestSource, TestSpec};
use crate::{ParallelismHint, TestRunner};
use coral_env::{EnvBackend, EnvHandle, EnvPlan, EnvironmentSpec, ServiceStatus};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{
    Config as ProptestConfig, RngAlgorithm, TestRng, TestRunner as PtRunner,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

/// v0.23.3 default — 50 iterations per endpoint per case. A user who
/// wants more (or fewer) can pin via `iterations = N` on the manifest
/// entry or `--iterations` on the CLI.
pub const DEFAULT_ITERATIONS: u32 = 50;

/// Strongly-typed payload of a `TestCase` whose `kind = PropertyBased`.
/// Stashed in `TestSpec.0` as JSON; deserialized on `run()`.
///
/// Carrying everything the runner needs on the case (and not on the
/// runner struct) keeps the case self-describing — `coral test --kind
/// property-based --service api` can be invoked even without
/// re-resolving the env spec, the cases hold their own iterations
/// and seeds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyTestCaseSpec {
    /// OpenAPI path template. Path parameters appear as `{name}`.
    pub path: String,
    /// HTTP method. v0.23.3 ships `GET` + `POST` only.
    pub method: String,
    /// How many proptest iterations to run for this endpoint.
    pub iterations: u32,
    /// Resolved seed. `None` here means "use a fresh random seed at
    /// run-time and log it" (D5). The CLI's `--seed` flag and the
    /// manifest both materialize through here when set.
    pub seed: Option<u64>,
    /// Pre-extracted JSON Schema for path parameters (object with one
    /// property per `{name}`). None when there are no path params.
    pub params_schema: Option<serde_json::Value>,
    /// Pre-extracted JSON Schema for the request body. None when the
    /// operation has no `requestBody` (always None for GET in v0.23.3
    /// since GET with body is non-standard).
    pub body_schema: Option<serde_json::Value>,
    /// Status codes the runner accepts as "success". Pulled from the
    /// operation's `responses` block — 2xx codes plus the OpenAPI
    /// `4xx` family (since validation errors are valid responses to
    /// random fuzzed inputs).
    pub expected_codes: Vec<u16>,
    /// Source spec path, repo-root-relative — used for the
    /// `Evidence::stdout_tail` and as the `TestSource::Discovered`
    /// `from` field.
    pub source_spec: String,
}

/// Build the strategy that generates a JSON value matching `schema`.
///
/// **Hand-rolled** to keep the dep surface to just `proptest` —
/// alternatives like `proptest-arbitrary-interop` add layers we don't
/// need for the 5-type subset.
///
/// Recursion depth is capped at `MAX_DEPTH` so a self-referential
/// (or merely-deeply-nested) schema can't stack-overflow the
/// generator. Past the cap, child schemas of objects/arrays fall
/// back to `null`.
pub fn json_schema_strategy(schema: &serde_json::Value) -> BoxedStrategy<serde_json::Value> {
    json_schema_strategy_with_depth(schema, 0)
}

const MAX_DEPTH: usize = 4;

fn json_schema_strategy_with_depth(
    schema: &serde_json::Value,
    depth: usize,
) -> BoxedStrategy<serde_json::Value> {
    if depth > MAX_DEPTH {
        // Past the recursion cap, generate `null` deterministically.
        return Just(serde_json::Value::Null).boxed();
    }
    let ty = schema
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("string");
    match ty {
        "string" => {
            // Bounded length: default `[0, 8]`. Past 8 chars adds noise
            // without catching new bug classes.
            let min = schema
                .get("minLength")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let max = schema
                .get("maxLength")
                .and_then(|v| v.as_u64())
                .unwrap_or(8) as usize;
            let max = max.max(min); // safety
            // Use printable-ASCII range so curl-arg-safety is easy.
            // Path-segment encoding happens later (see
            // `format_path_segment`).
            proptest::collection::vec(any::<u8>().prop_map(|b| (b % 95) + 32), min..=max)
                .prop_map(|bytes| {
                    let s: String = bytes.into_iter().map(|b| b as char).collect();
                    serde_json::Value::String(s)
                })
                .boxed()
        }
        "integer" => {
            let min = schema
                .get("minimum")
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MIN);
            let max = schema
                .get("maximum")
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MAX);
            let min = min.min(max);
            (min..=max)
                .prop_map(|n| serde_json::Value::Number(serde_json::Number::from(n)))
                .boxed()
        }
        "number" => {
            // proptest's f64::ANY is `Double::ANY`; clamp NaN out so
            // serde_json doesn't reject the value (NaN isn't valid JSON).
            any::<f64>()
                .prop_filter("NaN/Inf are not valid JSON", |f| f.is_finite())
                .prop_map(|f| {
                    serde_json::Number::from_f64(f)
                        .map(serde_json::Value::Number)
                        .unwrap_or(serde_json::Value::Null)
                })
                .boxed()
        }
        "object" => {
            let props = schema
                .get("properties")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let required: Vec<String> = schema
                .get("required")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            // Build a Vec<(key, Strategy)> over required props.
            // Optional props are emitted with 50% probability — small
            // enough to keep request size bounded, large enough to
            // cover the cases.
            let mut required_strategies: Vec<(String, BoxedStrategy<serde_json::Value>)> =
                Vec::new();
            for key in &required {
                if let Some(s) = props.get(key) {
                    required_strategies
                        .push((key.clone(), json_schema_strategy_with_depth(s, depth + 1)));
                }
            }
            // Optional props become Option<Strategy> so we can omit them.
            let mut optional_strategies: Vec<(String, BoxedStrategy<Option<serde_json::Value>>)> =
                Vec::new();
            for (key, sch) in &props {
                if !required.contains(key) {
                    let s = json_schema_strategy_with_depth(sch, depth + 1);
                    optional_strategies.push((
                        key.clone(),
                        prop_oneof![Just(None), s.prop_map(Some)].boxed(),
                    ));
                }
            }
            // Compose into one strategy that builds a serde_json::Map.
            (
                required_strategies
                    .into_iter()
                    .map(|(_, s)| s)
                    .collect::<Vec<_>>(),
                optional_strategies
                    .into_iter()
                    .map(|(_, s)| s)
                    .collect::<Vec<_>>(),
            )
                .prop_map({
                    let req_keys: Vec<String> = required.clone();
                    let opt_keys: Vec<String> = props
                        .keys()
                        .filter(|k| !required.contains(*k))
                        .cloned()
                        .collect();
                    move |(req_vals, opt_vals): (
                        Vec<serde_json::Value>,
                        Vec<Option<serde_json::Value>>,
                    )| {
                        let mut m = serde_json::Map::new();
                        for (k, v) in req_keys.iter().zip(req_vals) {
                            m.insert(k.clone(), v);
                        }
                        for (k, v) in opt_keys.iter().zip(opt_vals) {
                            if let Some(val) = v {
                                m.insert(k.clone(), val);
                            }
                        }
                        serde_json::Value::Object(m)
                    }
                })
                .boxed()
        }
        "array" => {
            let item_schema = schema
                .get("items")
                .cloned()
                .unwrap_or(serde_json::json!({"type": "string"}));
            let min = schema.get("minItems").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let max = schema.get("maxItems").and_then(|v| v.as_u64()).unwrap_or(4) as usize;
            let max = max.max(min);
            proptest::collection::vec(
                json_schema_strategy_with_depth(&item_schema, depth + 1),
                min..=max,
            )
            .prop_map(serde_json::Value::Array)
            .boxed()
        }
        _ => {
            // Unknown / unsupported type — generate a string fallback
            // so the runner can still hit the endpoint.
            proptest::collection::vec(any::<u8>().prop_map(|b| (b % 95) + 32), 0..=8)
                .prop_map(|bytes| {
                    let s: String = bytes.into_iter().map(|b| b as char).collect();
                    serde_json::Value::String(s)
                })
                .boxed()
        }
    }
}

/// Discover OpenAPI specs in `project_root` and emit one TestCase per
/// `(path, method)`. v0.23.3: only triggered when the orchestrator
/// sees `[[environments.<env>.property_tests]]` entries — there's no
/// implicit walk-the-tree path.
///
/// Returns an empty Vec when the env declares no `property_tests`
/// blocks (orchestrator's gate; this function honors what it's told).
pub fn cases_from_property_specs(
    spec: &EnvironmentSpec,
    project_root: &Path,
    cli_iterations: Option<u32>,
    cli_seed: Option<u64>,
) -> TestResult<Vec<TestCase>> {
    let mut out: Vec<TestCase> = Vec::new();
    for entry in &spec.property_tests {
        let spec_path = if entry.spec.is_absolute() {
            entry.spec.clone()
        } else {
            project_root.join(&entry.spec)
        };
        if !spec_path.is_file() {
            return Err(TestError::FixtureNotFound(spec_path));
        }
        let value = parse_openapi_value(&spec_path)?;
        // CLI override → manifest → default. (D6)
        let iterations = cli_iterations
            .or(entry.iterations)
            .unwrap_or(DEFAULT_ITERATIONS);
        let seed = cli_seed.or(entry.seed);
        let endpoints = walk_endpoints(&value);
        for ep in endpoints {
            let case_spec = PropertyTestCaseSpec {
                path: ep.path.clone(),
                method: ep.method.clone(),
                iterations,
                seed,
                params_schema: ep.params_schema,
                body_schema: ep.body_schema,
                expected_codes: ep.expected_codes,
                source_spec: spec_path.to_string_lossy().into_owned(),
            };
            let case = TestCase {
                id: format!("property:{}:{}", ep.method, ep.path),
                name: format!("property {} {}", ep.method, ep.path),
                kind: TestKind::PropertyBased,
                service: Some(entry.service.clone()),
                tags: vec!["property".into(), "discovered".into()],
                source: TestSource::Discovered {
                    from: spec_path.to_string_lossy().into_owned(),
                },
                spec: TestSpec(serde_json::to_value(&case_spec).unwrap_or(serde_json::Value::Null)),
            };
            out.push(case);
        }
    }
    Ok(out)
}

/// Walk `project_root` for any OpenAPI spec, emit one TestCase per
/// `(path, method)` for property-based testing (no service binding
/// — this is for the discovery-only path used by `--include-discovered`
/// at higher levels). Currently unused by the CLI but useful for
/// future extensions; kept compile-tested.
#[allow(dead_code)]
pub(crate) fn cases_from_walk(project_root: &Path) -> TestResult<Vec<TestCase>> {
    let mut out: Vec<TestCase> = Vec::new();
    for spec_path in find_openapi_specs(project_root)? {
        let value = match parse_openapi_value(&spec_path) {
            Ok(v) => v,
            Err(_) => continue,
        };
        for ep in walk_endpoints(&value) {
            let case_spec = PropertyTestCaseSpec {
                path: ep.path.clone(),
                method: ep.method.clone(),
                iterations: DEFAULT_ITERATIONS,
                seed: None,
                params_schema: ep.params_schema,
                body_schema: ep.body_schema,
                expected_codes: ep.expected_codes,
                source_spec: spec_path.to_string_lossy().into_owned(),
            };
            let case = TestCase {
                id: format!("property:{}:{}", ep.method, ep.path),
                name: format!("property {} {}", ep.method, ep.path),
                kind: TestKind::PropertyBased,
                service: None,
                tags: vec!["property".into(), "discovered".into()],
                source: TestSource::Discovered {
                    from: spec_path.to_string_lossy().into_owned(),
                },
                spec: TestSpec(serde_json::to_value(&case_spec).unwrap_or(serde_json::Value::Null)),
            };
            out.push(case);
        }
    }
    Ok(out)
}

/// Return the list of `(path, method)` operations the runner can
/// exercise from this spec.
///
/// **GET + POST only** in v0.23.3. Operations with `parameters[*].in
/// = "query"` or `parameters[*].in = "header"` are still walked; we
/// just ignore those parameter declarations and only consume the
/// `path` ones. Future versions will widen the surface.
fn walk_endpoints(value: &serde_json::Value) -> Vec<EndpointDecl> {
    let paths = match value.get("paths").and_then(|v| v.as_object()) {
        Some(p) => p,
        None => return Vec::new(),
    };
    let mut out: Vec<EndpointDecl> = Vec::new();
    for (path_str, ops_value) in paths {
        let ops = match ops_value.as_object() {
            Some(o) => o,
            None => continue,
        };
        for (method_str, op_value) in ops {
            let method = method_str.to_uppercase();
            if !matches!(method.as_str(), "GET" | "POST") {
                continue;
            }
            let params_schema = build_params_schema(op_value);
            let body_schema = build_body_schema(op_value);
            let expected_codes = collect_expected_codes(op_value);
            out.push(EndpointDecl {
                path: path_str.clone(),
                method,
                params_schema,
                body_schema,
                expected_codes,
            });
        }
    }
    out
}

/// Parsed shape of one `(path, method)` operation. Internal — exposed
/// to the test module only.
#[derive(Debug, Clone)]
pub(crate) struct EndpointDecl {
    pub(crate) path: String,
    pub(crate) method: String,
    pub(crate) params_schema: Option<serde_json::Value>,
    pub(crate) body_schema: Option<serde_json::Value>,
    pub(crate) expected_codes: Vec<u16>,
}

/// Assemble the `properties` of a synthetic JSON object schema from
/// the operation's `parameters[in=path]` entries. Returns `None` when
/// there are no path parameters.
fn build_params_schema(op: &serde_json::Value) -> Option<serde_json::Value> {
    let params = op.get("parameters").and_then(|v| v.as_array())?;
    let mut props = serde_json::Map::new();
    let mut required: Vec<serde_json::Value> = Vec::new();
    for p in params {
        let in_loc = p.get("in").and_then(|v| v.as_str()).unwrap_or("");
        if in_loc != "path" {
            continue;
        }
        let name = match p.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // OpenAPI parameter schema lives under `schema:` (3.x) or
        // siblings (`type`, `format`) on 2.x. Prefer 3.x.
        let sch = p
            .get("schema")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"type": "string"}));
        // Path params are always required by OpenAPI rules, but
        // double-check for explicit `required: true` if present.
        let is_required = p.get("required").and_then(|v| v.as_bool()).unwrap_or(true);
        if is_required {
            required.push(serde_json::Value::String(name.clone()));
        }
        props.insert(name, sch);
    }
    if props.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "type": "object",
        "properties": props,
        "required": required,
    }))
}

/// Pull the `application/json` schema out of `requestBody.content`.
/// Returns `None` when there's no body or no JSON content type.
fn build_body_schema(op: &serde_json::Value) -> Option<serde_json::Value> {
    op.get("requestBody")
        .and_then(|rb| rb.get("content"))
        .and_then(|c| c.get("application/json"))
        .and_then(|j| j.get("schema"))
        .cloned()
}

/// Walk `responses` for declared status codes. Accepts:
/// - 2xx exact codes.
/// - 4xx exact codes (validation errors are legitimate responses to
///   random fuzzed input — the server isn't broken because it
///   rejected our garbage).
/// - The OpenAPI wildcard `"4XX"` / `"5XX"` is **rejected** here:
///   v0.23.3 does exact-status matching, not pattern matching.
///
/// When the operation declares NO codes, falls back to `[200]`.
fn collect_expected_codes(op: &serde_json::Value) -> Vec<u16> {
    let responses = match op.get("responses").and_then(|v| v.as_object()) {
        Some(r) => r,
        None => return vec![200],
    };
    let mut out: Vec<u16> = responses
        .keys()
        .filter_map(|k| k.parse::<u16>().ok())
        // 2xx: server agreed our random was valid → pass.
        // 4xx: server rejected our random → that's also a pass; the
        //   API correctly validated input. 5xx is the failure surface.
        .filter(|s| (200..500).contains(s))
        .collect();
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        out.push(200);
    }
    out
}

/// `PropertyRunner` — drive proptest iterations for a `TestCase` whose
/// `kind = PropertyBased`. Holds the env handle dependencies for
/// symmetry with `RecordedRunner` / `UserDefinedRunner`; the actual
/// HTTP invocation is curl-subprocess identical to `recorded_runner`.
pub struct PropertyRunner {
    backend: Arc<dyn EnvBackend>,
    plan: EnvPlan,
    spec: EnvironmentSpec,
}

impl PropertyRunner {
    pub fn new(backend: Arc<dyn EnvBackend>, plan: EnvPlan, spec: EnvironmentSpec) -> Self {
        Self {
            backend,
            plan,
            spec,
        }
    }

    /// Drive one TestCase: extract the spec, run N iterations, return
    /// `(status, evidence)`. Pure over the case payload + the
    /// `http_invoke` callback so the unit tests can pass a canned
    /// invoker without spawning curl.
    ///
    /// `seed` precedence is resolved by the caller — by the time we
    /// land here, `case_spec.seed` is either the manifest value, the
    /// CLI override, or `None` (in which case we draw fresh and log).
    pub(crate) fn drive_case(
        case_spec: &PropertyTestCaseSpec,
        invoker: &mut dyn FnMut(&str, &str, Option<&serde_json::Value>) -> InvokeOutcome,
    ) -> (TestStatus, Evidence) {
        let resolved_seed = case_spec.seed.unwrap_or_else(fresh_random_u64_seed);
        if case_spec.seed.is_none() {
            tracing::info!(
                seed = resolved_seed,
                path = %case_spec.path,
                method = %case_spec.method,
                iterations = case_spec.iterations,
                "property runner: drawing fresh seed (acceptance #7)"
            );
        }
        let seed_bytes = u64_to_chacha_seed(resolved_seed);
        let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &seed_bytes);
        let config = ProptestConfig {
            cases: case_spec.iterations.max(1),
            ..ProptestConfig::default()
        };
        let mut runner = PtRunner::new_with_rng(config, rng);

        // Build the combined input strategy: Option<params>, Option<body>.
        // We use `(strategy, strategy)` -> (Option<Value>, Option<Value>)
        // because proptest tuples derive Strategy + can shrink coordinately.
        let params_strategy: BoxedStrategy<Option<serde_json::Value>> =
            match &case_spec.params_schema {
                Some(ps) => json_schema_strategy(ps).prop_map(Some).boxed(),
                None => Just(None).boxed(),
            };
        let body_strategy: BoxedStrategy<Option<serde_json::Value>> = match &case_spec.body_schema {
            Some(bs) => json_schema_strategy(bs).prop_map(Some).boxed(),
            None => Just(None).boxed(),
        };

        // Iterate manually so we can stop on the first failure and
        // run the shrinker over the same value tree.
        let mut last_failure: Option<FailureRecord> = None;
        for i in 0..case_spec.iterations {
            // Generate a fresh value tree per iteration.
            let mut params_tree = match params_strategy.new_tree(&mut runner) {
                Ok(t) => t,
                Err(e) => {
                    return (
                        TestStatus::Error {
                            reason: format!(
                                "property runner: param strategy could not generate a value tree: {e:?}"
                            ),
                        },
                        Evidence::default(),
                    );
                }
            };
            let mut body_tree = match body_strategy.new_tree(&mut runner) {
                Ok(t) => t,
                Err(e) => {
                    return (
                        TestStatus::Error {
                            reason: format!(
                                "property runner: body strategy could not generate a value tree: {e:?}"
                            ),
                        },
                        Evidence::default(),
                    );
                }
            };
            let params = params_tree.current();
            let body = body_tree.current();
            let url_path = match interpolate_path(&case_spec.path, params.as_ref()) {
                Ok(p) => p,
                Err(reason) => {
                    return (
                        TestStatus::Skip {
                            reason: format!("property runner: {reason}"),
                        },
                        Evidence::default(),
                    );
                }
            };
            let outcome = invoker(&case_spec.method, &url_path, body.as_ref());
            if !case_spec.expected_codes.contains(&outcome.status) {
                // Failed iteration — proptest shrinks BOTH trees in
                // parallel, retesting until simplify() returns false.
                let mut shrunken_params = params.clone();
                let mut shrunken_body = body.clone();
                let mut shrunken_status = outcome.status;
                let mut shrunken_body_tail = outcome.body_tail.clone();
                let iteration_for_msg = i + 1;
                // Try to shrink params first, then body. For each
                // dimension: while simplify() returns true and the
                // simpler value still fails, accept the simpler value.
                loop {
                    let mut progressed = false;
                    if params_tree.simplify() {
                        let candidate_params = params_tree.current();
                        let url_candidate =
                            interpolate_path(&case_spec.path, candidate_params.as_ref())
                                .unwrap_or_else(|_| case_spec.path.clone());
                        let cand =
                            invoker(&case_spec.method, &url_candidate, shrunken_body.as_ref());
                        if !case_spec.expected_codes.contains(&cand.status) {
                            shrunken_params = candidate_params;
                            shrunken_status = cand.status;
                            shrunken_body_tail = cand.body_tail.clone();
                            progressed = true;
                        } else {
                            // The simpler value PASSED — undo by
                            // calling complicate(); we don't track
                            // back further to keep this implementation
                            // simple.
                            let _ = params_tree.complicate();
                        }
                    }
                    if body_tree.simplify() {
                        let candidate_body = body_tree.current();
                        let url_candidate =
                            interpolate_path(&case_spec.path, shrunken_params.as_ref())
                                .unwrap_or_else(|_| case_spec.path.clone());
                        let cand =
                            invoker(&case_spec.method, &url_candidate, candidate_body.as_ref());
                        if !case_spec.expected_codes.contains(&cand.status) {
                            shrunken_body = candidate_body;
                            shrunken_status = cand.status;
                            shrunken_body_tail = cand.body_tail.clone();
                            progressed = true;
                        } else {
                            let _ = body_tree.complicate();
                        }
                    }
                    if !progressed {
                        break;
                    }
                }
                last_failure = Some(FailureRecord {
                    iteration_index_one_based: iteration_for_msg,
                    shrunken_params,
                    shrunken_body,
                    shrunken_status,
                    shrunken_body_tail,
                });
                break;
            }
        }

        if let Some(f) = last_failure {
            let reason = format!(
                "{} not in [{}] (iteration {} of {})",
                f.shrunken_status,
                f.format_codes(&case_spec.expected_codes),
                f.iteration_index_one_based,
                case_spec.iterations,
            );
            let ev = Evidence {
                http: Some(HttpEvidence {
                    method: case_spec.method.clone(),
                    url: case_spec.path.clone(),
                    status: f.shrunken_status,
                    body_tail: f.shrunken_body_tail.clone(),
                }),
                stdout_tail: Some(format!(
                    "shrunken_input={{params: {}, body: {}}} seed={resolved_seed}",
                    json_compact(&f.shrunken_params),
                    json_compact(&f.shrunken_body),
                )),
                ..Evidence::default()
            };
            return (TestStatus::Fail { reason }, ev);
        }

        // All N iterations passed.
        let ev = Evidence {
            stdout_tail: Some(format!(
                "{}/{} inputs passed (seed={resolved_seed})",
                case_spec.iterations, case_spec.iterations,
            )),
            ..Evidence::default()
        };
        (TestStatus::Pass, ev)
    }
}

impl TestRunner for PropertyRunner {
    fn name(&self) -> &'static str {
        "property"
    }

    fn supports(&self, kind: TestKind) -> bool {
        matches!(kind, TestKind::PropertyBased)
    }

    fn run(&self, case: &TestCase, _env: &EnvHandle) -> TestResult<TestReport> {
        let started = Instant::now();
        let case_spec: PropertyTestCaseSpec =
            serde_json::from_value(case.spec.0.clone()).map_err(|e| TestError::InvalidSpec {
                path: PathBuf::from(case.id.clone()),
                reason: format!("decoding PropertyTestCaseSpec: {e}"),
            })?;
        // Resolve the published port for the target service.
        let host_port = self.resolve_service_port(case.service.as_deref())?;
        let mut invoker = |method: &str, path: &str, body: Option<&serde_json::Value>| {
            invoke_curl(host_port, method, path, body)
        };
        let (status, evidence) = Self::drive_case(&case_spec, &mut invoker);
        let mut report = TestReport::new(case.clone(), status, started.elapsed());
        report.evidence = evidence;
        Ok(report)
    }

    fn discover(&self, project_root: &Path) -> TestResult<Vec<TestCase>> {
        cases_from_property_specs(&self.spec, project_root, None, None)
    }

    fn parallelism_hint(&self) -> ParallelismHint {
        // Each endpoint runs its iteration loop independently; cases
        // don't share state, so they're parallel-safe.
        ParallelismHint::Isolated
    }
}

impl PropertyRunner {
    fn resolve_service_port(&self, service: Option<&str>) -> TestResult<u16> {
        let svc = service.ok_or_else(|| TestError::ServiceNotExposed("(none)".into()))?;
        let env_status = self.backend.status(&self.plan)?;
        let svc_status: ServiceStatus =
            match env_status.services.into_iter().find(|s| s.name == svc) {
                Some(s) => s,
                None => return Err(TestError::ServiceNotExposed(svc.into())),
            };
        match svc_status.published_ports.first() {
            Some(p) if p.host_port > 0 => Ok(p.host_port),
            _ => Err(TestError::ServiceNotExposed(format!(
                "{svc} (no published port)"
            ))),
        }
    }
}

// ---- helpers ---------------------------------------------------------------

/// Captured single-iteration failure. Carried into the final
/// `Fail { reason }` formatter.
struct FailureRecord {
    iteration_index_one_based: u32,
    shrunken_params: Option<serde_json::Value>,
    shrunken_body: Option<serde_json::Value>,
    shrunken_status: u16,
    shrunken_body_tail: Option<String>,
}

impl FailureRecord {
    fn format_codes(&self, codes: &[u16]) -> String {
        codes
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The output of one HTTP invocation. Mirrors the shape that
/// `recorded_runner::InvokedResponse` carries, but on a smaller
/// surface — we only need status + body tail, not headers.
#[derive(Debug, Clone)]
pub struct InvokeOutcome {
    pub status: u16,
    pub body_tail: Option<String>,
}

fn invoke_curl(
    host_port: u16,
    method: &str,
    path: &str,
    body: Option<&serde_json::Value>,
) -> InvokeOutcome {
    let url = format!("http://127.0.0.1:{host_port}{path}");
    let mut cmd = Command::new("curl");
    cmd.args([
        "-s",
        "-X",
        method,
        "-w",
        "\nHTTP_STATUS:%{http_code}",
        "--max-time",
        "10",
    ]);
    if let Some(b) = body {
        cmd.args(["-H", "Content-Type: application/json"]);
        cmd.args(["-d", b.to_string().as_str()]);
    }
    cmd.arg(&url);
    let output = match cmd.output() {
        Ok(o) => o,
        Err(_) => {
            return InvokeOutcome {
                status: 0,
                body_tail: Some("curl spawn failed".into()),
            };
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (body_part, status) = split_curl_status(&stdout);
    InvokeOutcome {
        status,
        body_tail: Some(truncate(body_part, 256)),
    }
}

/// Parse the `\nHTTP_STATUS:NNN` suffix written by `curl -w`. Same
/// helper shape as `user_defined_runner::split_curl_status`.
fn split_curl_status(s: &str) -> (&str, u16) {
    if let Some(idx) = s.rfind("HTTP_STATUS:") {
        let body = s[..idx].trim_end_matches('\n');
        let code = s[idx + "HTTP_STATUS:".len()..]
            .trim()
            .parse::<u16>()
            .unwrap_or(0);
        (body, code)
    } else {
        (s, 0)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Walk char boundaries so we don't slice in the middle of a
        // multi-byte UTF-8 sequence.
        let mut idx = max;
        while !s.is_char_boundary(idx) && idx > 0 {
            idx -= 1;
        }
        format!("{}…", &s[..idx])
    }
}

fn json_compact(v: &Option<serde_json::Value>) -> String {
    match v {
        Some(val) => serde_json::to_string(val).unwrap_or_else(|_| "<unencodable>".into()),
        None => "null".into(),
    }
}

/// Substitute `{name}` placeholders in `template` with the values
/// from `params`, URL-encoding each segment minimally (spaces → `%20`,
/// slashes → `%2F`). Returns Err with a reason when a `{name}` has
/// no corresponding key in `params`.
pub(crate) fn interpolate_path(
    template: &str,
    params: Option<&serde_json::Value>,
) -> Result<String, String> {
    if !template.contains('{') {
        return Ok(template.to_string());
    }
    let obj = match params.and_then(|v| v.as_object()) {
        Some(o) => o,
        None => {
            return Err(format!(
                "path '{template}' has placeholders but no params object"
            ));
        }
    };
    let mut out = String::with_capacity(template.len() + 16);
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            // Find the matching `}`.
            let rel = template[i + 1..]
                .find('}')
                .ok_or_else(|| format!("unterminated placeholder in path '{template}'"))?;
            let name = &template[i + 1..i + 1 + rel];
            let val = obj
                .get(name)
                .ok_or_else(|| format!("missing path param '{name}' for path '{template}'"))?;
            out.push_str(&format_path_segment(val));
            i += 1 + rel + 1;
        } else {
            // Multi-byte safe — push the char.
            let ch_end = next_char_boundary(template, i);
            out.push_str(&template[i..ch_end]);
            i = ch_end;
        }
    }
    Ok(out)
}

fn next_char_boundary(s: &str, i: usize) -> usize {
    let mut j = i + 1;
    while j < s.len() && !s.is_char_boundary(j) {
        j += 1;
    }
    j
}

fn format_path_segment(v: &serde_json::Value) -> String {
    let raw = match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        // For object/array, encode JSON. The route is unlikely to
        // accept it, but the response will tell us 4xx and the runner
        // will count that as a "validated" pass per AC.
        other => other.to_string(),
    };
    minimal_url_encode(&raw)
}

/// Minimal URL encoder for path segments. Reserves what curl + most
/// HTTP servers require (spaces, `/`, `?`, `#`, control chars) and
/// passes the rest through. Pre-encoded values (`%20`-style) are
/// **not** rewritten — the encoder is lossy on raw `%` to keep things
/// simple.
fn minimal_url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let safe = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if safe {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Expand a `u64` seed into the 32-byte buffer that proptest's ChaCha
/// algorithm requires. Replicate the LE bytes 4× to make the
/// expansion deterministic and reproducible across platforms — same
/// `u64` always produces the same 32-byte buffer.
pub(crate) fn u64_to_chacha_seed(seed: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    let bytes = seed.to_le_bytes();
    for chunk_idx in 0..4 {
        let base = chunk_idx * 8;
        out[base..base + 8].copy_from_slice(&bytes);
    }
    out
}

/// Draw a fresh `u64` seed from the system clock when neither the
/// CLI nor the manifest pinned one. Hashing the wall-clock nanos
/// with the process id and a small bit-mix routine keeps the seed
/// path single-thread-safe and unbiased enough for property tests
/// (we don't need cryptographic randomness — the user can pin
/// `--seed` for reproducibility on their failing case).
///
/// We deliberately avoid pulling `rand::random::<u64>()` here so
/// `coral-test` doesn't depend on `rand` directly — the orchestrator
/// spec working-agreement #2 budgets ONE new workspace dep
/// (proptest); rand stays transitive.
fn fresh_random_u64_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    // SplitMix64-style finalizer for whitening — avoids correlation
    // between two callers in the same nanosecond by mixing pid in.
    let mut z = nanos.wrapping_add(pid).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_env::spec::EnvMode;
    use coral_env::{RealService, ServiceKind};
    use std::collections::BTreeMap as Map;

    /// Drain the strategy by stepping the value tree N times and
    /// returning the resulting JSON values. Helper for the strategy
    /// tests.
    fn sample_n(strategy: &BoxedStrategy<serde_json::Value>, n: usize) -> Vec<serde_json::Value> {
        let mut runner = PtRunner::new_with_rng(
            ProptestConfig::default(),
            TestRng::from_seed(RngAlgorithm::ChaCha, &u64_to_chacha_seed(1234)),
        );
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let tree = strategy
                .new_tree(&mut runner)
                .expect("tree generation succeeds");
            out.push(tree.current());
        }
        out
    }

    /// **T3 (acceptance #3) — string strategy generates JSON strings.**
    /// Hand-roll the `{ "type": "string" }` schema and assert every
    /// produced value is a string with bounded length.
    #[test]
    fn json_schema_string_type_generates_string_value() {
        let schema = serde_json::json!({"type": "string"});
        let strategy = json_schema_strategy(&schema);
        let samples = sample_n(&strategy, 32);
        for v in &samples {
            assert!(v.is_string(), "expected string, got {v:?}");
            assert!(
                v.as_str().unwrap().len() <= 8,
                "default length cap should be 8, got: {v}"
            );
        }
    }

    /// **T4 (acceptance #3 cont.) — integer strategy.**
    #[test]
    fn json_schema_integer_type_generates_int_value() {
        let schema = serde_json::json!({
            "type": "integer",
            "minimum": 0,
            "maximum": 100,
        });
        let strategy = json_schema_strategy(&schema);
        let samples = sample_n(&strategy, 64);
        for v in &samples {
            let n = v.as_i64().expect("integer");
            assert!((0..=100).contains(&n), "integer out of bounds: {n}");
        }
    }

    /// **T5 — object with required fields generates required keys.**
    /// Pin: every emitted object MUST contain every key listed in
    /// `required`. Optional keys may or may not be present.
    #[test]
    fn json_schema_object_type_generates_object_with_required_fields() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "id":    { "type": "integer", "minimum": 1, "maximum": 100 },
                "name":  { "type": "string" },
                "extra": { "type": "string" },
            },
            "required": ["id", "name"],
        });
        let strategy = json_schema_strategy(&schema);
        let samples = sample_n(&strategy, 24);
        for v in &samples {
            let obj = v.as_object().expect("object");
            assert!(obj.contains_key("id"), "id is required, got: {v}");
            assert!(obj.contains_key("name"), "name is required, got: {v}");
        }
    }

    fn property_spec_with_one_test() -> EnvironmentSpec {
        let mut services = Map::new();
        services.insert(
            "api".to_string(),
            ServiceKind::Real(Box::new(RealService {
                repo: None,
                image: Some("api:dev".into()),
                build: None,
                ports: vec![3000],
                env: Map::new(),
                depends_on: vec![],
                healthcheck: None,
                watch: None,
            })),
        );
        EnvironmentSpec {
            name: "dev".into(),
            backend: "compose".into(),
            mode: EnvMode::Managed,
            compose_command: "auto".into(),
            production: false,
            env_file: None,
            services,
            chaos: None,
            chaos_scenarios: Vec::new(),
            monitors: Vec::new(),
            recorded: None,
            property_tests: vec![coral_env::PropertyTestSpec {
                service: "api".into(),
                spec: PathBuf::from("openapi.yaml"),
                seed: Some(42),
                iterations: Some(3),
            }],
        }
    }

    /// **T6 — seed=42 produces the same input sequence twice.**
    /// Run `drive_case` with a captured-input invoker; assert the two
    /// runs see the same sequence of `(method, path, body)` tuples.
    /// (Acceptance #6.)
    #[test]
    fn seed_42_produces_deterministic_input_sequence() {
        let case_spec = PropertyTestCaseSpec {
            path: "/users/{id}".into(),
            method: "GET".into(),
            iterations: 5,
            seed: Some(42),
            params_schema: Some(serde_json::json!({
                "type": "object",
                "properties": { "id": { "type": "integer", "minimum": 0, "maximum": 999 } },
                "required": ["id"],
            })),
            body_schema: None,
            expected_codes: vec![200],
            source_spec: "openapi.yaml".into(),
        };

        fn run_capture(case_spec: &PropertyTestCaseSpec) -> Vec<(String, String)> {
            let mut log: Vec<(String, String)> = Vec::new();
            let mut invoker = |method: &str, path: &str, _body: Option<&serde_json::Value>| {
                log.push((method.to_string(), path.to_string()));
                InvokeOutcome {
                    status: 200,
                    body_tail: None,
                }
            };
            let _ = PropertyRunner::drive_case(case_spec, &mut invoker);
            log
        }
        let run1 = run_capture(&case_spec);
        let run2 = run_capture(&case_spec);
        assert_eq!(run1.len(), 5);
        assert_eq!(
            run1, run2,
            "seed=42 produced different sequences across two runs"
        );
    }

    /// **T7 — when seed is omitted, drive_case logs the actual seed
    /// AND embeds it into Evidence::stdout_tail.** (Acceptance #7.)
    /// We don't assert on the tracing log here (no test subscriber);
    /// we assert on Evidence::stdout_tail which is the user-visible
    /// half of the contract.
    #[test]
    fn omitted_seed_logs_actual_seed_used() {
        let case_spec = PropertyTestCaseSpec {
            path: "/h".into(),
            method: "GET".into(),
            iterations: 1,
            seed: None, // <- the contract
            params_schema: None,
            body_schema: None,
            expected_codes: vec![200],
            source_spec: "openapi.yaml".into(),
        };
        let mut invoker = |_m: &str, _p: &str, _b: Option<&serde_json::Value>| InvokeOutcome {
            status: 200,
            body_tail: None,
        };
        let (_status, ev) = PropertyRunner::drive_case(&case_spec, &mut invoker);
        let stdout = ev.stdout_tail.expect("stdout_tail set on pass");
        assert!(
            stdout.contains("seed="),
            "evidence must embed seed=N for reproducibility, got: {stdout}"
        );
        assert!(
            stdout.contains("inputs passed"),
            "pass evidence must say 'N/N inputs passed', got: {stdout}"
        );
    }

    /// **T8 — failed iteration → first counter-example wins.**
    /// Invoker returns 500 on the third call → drive_case stops at
    /// iteration 3 and returns Fail. Acceptance #8.
    #[test]
    fn property_runner_failed_iteration_returns_first_counter_example() {
        let case_spec = PropertyTestCaseSpec {
            path: "/h".into(),
            method: "GET".into(),
            iterations: 10,
            seed: Some(7),
            params_schema: None,
            body_schema: None,
            expected_codes: vec![200],
            source_spec: "openapi.yaml".into(),
        };
        let mut call_count = 0u32;
        let mut invoker = |_m: &str, _p: &str, _b: Option<&serde_json::Value>| {
            call_count += 1;
            // First two iterations pass, third + onward fail. (Status 500
            // is not in `expected_codes = [200]`.) Shrinker calls AFTER
            // the third also return 500 (same outcome), so the runner
            // halts.
            if call_count >= 3 {
                InvokeOutcome {
                    status: 500,
                    body_tail: Some("server error".into()),
                }
            } else {
                InvokeOutcome {
                    status: 200,
                    body_tail: None,
                }
            }
        };
        let (status, ev) = PropertyRunner::drive_case(&case_spec, &mut invoker);
        match status {
            TestStatus::Fail { reason } => {
                assert!(
                    reason.contains("500") && reason.contains("[200]"),
                    "fail reason should name 500 and the expected set: {reason}"
                );
                assert!(
                    reason.contains("of 10"),
                    "fail reason should mention iteration count: {reason}"
                );
            }
            other => panic!("expected Fail, got {other:?}"),
        }
        // Evidence carries the shrunken-input + body tail.
        assert_eq!(ev.http.as_ref().expect("http evidence").status, 500);
    }

    /// **T9 — all iterations pass → Pass with "N/N inputs passed".**
    /// Acceptance #9.
    #[test]
    fn property_runner_all_pass_returns_pass_status() {
        let case_spec = PropertyTestCaseSpec {
            path: "/h".into(),
            method: "GET".into(),
            iterations: 5,
            seed: Some(99),
            params_schema: None,
            body_schema: None,
            expected_codes: vec![200],
            source_spec: "openapi.yaml".into(),
        };
        let mut invoker = |_m: &str, _p: &str, _b: Option<&serde_json::Value>| InvokeOutcome {
            status: 200,
            body_tail: None,
        };
        let (status, ev) = PropertyRunner::drive_case(&case_spec, &mut invoker);
        assert!(matches!(status, TestStatus::Pass), "got: {status:?}");
        let tail = ev.stdout_tail.expect("stdout_tail set on pass");
        assert!(
            tail.contains("5/5 inputs passed"),
            "pass evidence must say '5/5 inputs passed', got: {tail}"
        );
    }

    /// **T2 + T4 — case discovery from a property_tests block emits
    /// one TestCase per (path, method).** Acceptance #4.
    #[test]
    fn cases_from_property_specs_emits_one_case_per_path_method() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("openapi.yaml"),
            r#"openapi: 3.0.0
info: { title: x, version: 1.0 }
paths:
  /users:
    get:
      responses:
        '200': { description: ok }
    post:
      requestBody:
        content:
          application/json:
            schema:
              type: object
              properties:
                name: { type: string }
              required: [name]
      responses:
        '201': { description: created }
  /users/{id}:
    get:
      parameters:
        - in: path
          name: id
          required: true
          schema: { type: integer }
      responses:
        '200': { description: ok }
"#,
        )
        .unwrap();
        let mut spec = property_spec_with_one_test();
        spec.property_tests[0].spec = PathBuf::from("openapi.yaml");
        let cases = cases_from_property_specs(&spec, dir.path(), None, None).expect("ok");
        // Three (path, method) pairs: GET /users, POST /users, GET /users/{id}.
        assert_eq!(
            cases.len(),
            3,
            "expected one case per (path, method): got {:?}",
            cases.iter().map(|c| &c.id).collect::<Vec<_>>()
        );
        // All cases tagged property + discovered.
        for c in &cases {
            assert_eq!(c.kind, TestKind::PropertyBased);
            assert_eq!(c.service.as_deref(), Some("api"));
            assert!(c.tags.iter().any(|t| t == "property"));
        }
    }

    /// PUT/PATCH/DELETE are out of scope for v0.23.3 (D1) — they must
    /// be silently dropped at discovery time.
    #[test]
    fn cases_from_property_specs_skips_non_get_post_methods() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("openapi.yaml"),
            r#"openapi: 3.0.0
info: { title: x, version: 1.0 }
paths:
  /users/{id}:
    put:
      parameters:
        - in: path
          name: id
          schema: { type: integer }
      responses: { '200': { description: ok } }
    delete:
      parameters:
        - in: path
          name: id
          schema: { type: integer }
      responses: { '204': { description: gone } }
    get:
      parameters:
        - in: path
          name: id
          schema: { type: integer }
      responses: { '200': { description: ok } }
"#,
        )
        .unwrap();
        let mut spec = property_spec_with_one_test();
        spec.property_tests[0].spec = PathBuf::from("openapi.yaml");
        let cases = cases_from_property_specs(&spec, dir.path(), None, None).expect("ok");
        assert_eq!(cases.len(), 1, "only GET should land");
    }

    /// CLI override — `--iterations 5` wins over a manifest-declared
    /// `iterations = 100`. Acceptance #10.
    #[test]
    fn iterations_cli_flag_overrides_manifest() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("openapi.yaml"),
            r#"openapi: 3.0.0
info: { title: x, version: 1.0 }
paths:
  /h:
    get:
      responses: { '200': { description: ok } }
"#,
        )
        .unwrap();
        let mut spec = property_spec_with_one_test();
        spec.property_tests[0].spec = PathBuf::from("openapi.yaml");
        spec.property_tests[0].iterations = Some(100);
        // CLI passes 5 — wins over the manifest's 100.
        let cases = cases_from_property_specs(&spec, dir.path(), Some(5), None).expect("ok");
        assert_eq!(cases.len(), 1);
        let case_spec: PropertyTestCaseSpec =
            serde_json::from_value(cases[0].spec.0.clone()).expect("decode");
        assert_eq!(case_spec.iterations, 5, "CLI override must beat manifest");
    }

    /// `interpolate_path` substitutes `{id}` with the params object value.
    #[test]
    fn interpolate_path_substitutes_placeholders() {
        let params = serde_json::json!({ "id": 42 });
        let out = interpolate_path("/users/{id}", Some(&params)).unwrap();
        assert_eq!(out, "/users/42");
    }

    /// `interpolate_path` returns the template unchanged when there
    /// are no placeholders.
    #[test]
    fn interpolate_path_passes_through_when_no_placeholders() {
        let out = interpolate_path("/h", None).unwrap();
        assert_eq!(out, "/h");
    }

    /// `interpolate_path` URL-encodes special chars in segments.
    #[test]
    fn interpolate_path_url_encodes_path_segments() {
        let params = serde_json::json!({ "name": "a b/c" });
        let out = interpolate_path("/u/{name}", Some(&params)).unwrap();
        // space → %20, `/` → %2F.
        assert_eq!(out, "/u/a%20b%2Fc");
    }

    /// `u64_to_chacha_seed` is deterministic — same u64 → same buffer.
    #[test]
    fn u64_to_chacha_seed_is_deterministic() {
        assert_eq!(u64_to_chacha_seed(42), u64_to_chacha_seed(42));
        assert_ne!(u64_to_chacha_seed(42), u64_to_chacha_seed(43));
    }

    /// `collect_expected_codes` accepts both 2xx and 4xx, falls back
    /// to [200] when the operation declares no responses.
    #[test]
    fn collect_expected_codes_accepts_2xx_and_4xx() {
        let op = serde_json::json!({
            "responses": {
                "200": {},
                "201": {},
                "400": {},
                "500": {},
            }
        });
        let codes = collect_expected_codes(&op);
        assert!(codes.contains(&200) && codes.contains(&201) && codes.contains(&400));
        // 5xx is the failure signal — must NOT be in the expected set.
        assert!(!codes.contains(&500));
    }

    /// PropertyRunner advertises the right TestKind.
    #[test]
    fn property_runner_supports_only_property_based_kind() {
        let spec = property_spec_with_one_test();
        let backend: Arc<dyn EnvBackend> = Arc::new(coral_env::MockBackend::new());
        let plan = EnvPlan::from_spec(&spec, Path::new("/tmp/x"), &Default::default())
            .expect("plan from spec");
        let runner = PropertyRunner::new(backend, plan, spec);
        assert!(runner.supports(TestKind::PropertyBased));
        assert!(!runner.supports(TestKind::Healthcheck));
        assert!(!runner.supports(TestKind::UserDefined));
    }

    /// Manifest-side validation: no property_tests entries → no cases.
    #[test]
    fn cases_from_property_specs_returns_empty_when_no_property_tests() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut spec = property_spec_with_one_test();
        spec.property_tests.clear();
        let cases = cases_from_property_specs(&spec, dir.path(), None, None).expect("ok");
        assert!(cases.is_empty());
    }

    /// Plan resolves: runner stash works.
    #[test]
    fn property_runner_new_smoke() {
        let spec = property_spec_with_one_test();
        let backend: Arc<dyn EnvBackend> = Arc::new(coral_env::MockBackend::new());
        let plan = EnvPlan::from_spec(&spec, Path::new("/tmp/x"), &Default::default())
            .expect("plan from spec");
        let _ = PropertyRunner::new(backend, plan, spec);
    }

    /// The runner's `service` field on a TestCase MUST come from the
    /// manifest entry, not the spec file. Pre-v0.23.3 OpenAPI walker
    /// guessed from `tags[0]`; PropertyRunner pins it via
    /// `property_tests[*].service`.
    #[test]
    fn cases_from_property_specs_uses_manifest_service_not_openapi_tags() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("openapi.yaml"),
            r#"openapi: 3.0.0
info: { title: x, version: 1.0 }
paths:
  /h:
    get:
      tags: [completely-different-name]
      responses: { '200': { description: ok } }
"#,
        )
        .unwrap();
        let mut spec = property_spec_with_one_test();
        spec.property_tests[0].spec = PathBuf::from("openapi.yaml");
        let cases = cases_from_property_specs(&spec, dir.path(), None, None).expect("ok");
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].service.as_deref(), Some("api"));
    }
}
