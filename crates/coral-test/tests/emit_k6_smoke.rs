//! Integration tests for `coral_test::emit_k6`.
//!
//! Six tests that pin the v0.22.2 acceptance criteria:
//! 1. `emit_k6_header_has_options_and_imports` (AC #3)
//! 2. `emit_k6_user_defined_two_step_suite_round_trips` — golden file (AC #5, #11)
//! 3. `emit_k6_skips_exec_steps_with_comment` (AC #5 mapping)
//! 4. `emit_k6_healthcheck_tcp_skipped` (AC #6)
//! 5. `emit_k6_service_base_uses_declared_port` (AC #4)
//! 6. `emit_k6_unknown_service_falls_back_to_BASE_with_todo` (D4 fallback)

use coral_env::EnvironmentSpec;
use coral_env::spec::{
    EnvMode, Healthcheck, HealthcheckKind, HealthcheckTiming, RealService, ServiceKind,
};
use coral_test::user_defined_runner::YamlSuite;
use coral_test::{TestCase, TestKind, TestSource, TestSpec};
use std::collections::BTreeMap;

fn spec_with_service(name: &str, port: u16, hc: Option<Healthcheck>) -> EnvironmentSpec {
    let mut services = BTreeMap::new();
    services.insert(
        name.to_string(),
        ServiceKind::Real(Box::new(RealService {
            repo: None,
            image: Some("alpine:latest".into()),
            build: None,
            ports: vec![port],
            env: BTreeMap::new(),
            depends_on: vec![],
            healthcheck: hc,
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
    }
}

fn user_defined_case(id: &str, suite: YamlSuite) -> TestCase {
    TestCase {
        id: id.to_string(),
        name: id.to_string(),
        kind: TestKind::UserDefined,
        service: suite.service.clone(),
        tags: suite.tags.clone(),
        source: TestSource::Inline,
        spec: TestSpec(serde_json::to_value(&suite).unwrap()),
    }
}

#[test]
fn emit_k6_header_has_options_and_imports() {
    let spec = spec_with_service("api", 3000, None);
    let out = coral_test::emit_k6(&[], &spec);
    // Acceptance #3: exactly one each.
    assert_eq!(out.script.matches("import http from 'k6/http';").count(), 1);
    assert_eq!(
        out.script
            .matches("import { check, sleep } from 'k6';")
            .count(),
        1
    );
    assert_eq!(out.script.matches("export const options").count(), 1);
    assert!(out.script.contains("vus: __ENV.VUS || 10"));
    assert!(out.script.contains("duration: __ENV.DURATION || '30s'"));
}

#[test]
fn emit_k6_user_defined_two_step_suite_round_trips() {
    let yaml = r#"
name: api smoke
service: api
tags: [smoke]
steps:
  - http: GET /users
    expect: { status: 200 }
  - http: POST /users
    body: { name: "alice" }
    expect: { status: 201, body_contains: "alice" }
"#;
    let suite: YamlSuite = serde_yaml_ng::from_str(yaml).unwrap();
    let case = user_defined_case("user-defined:api smoke", suite);
    let spec = spec_with_service("api", 3000, None);

    let out = coral_test::emit_k6(std::slice::from_ref(&case), &spec);
    // Determinism: re-running with the same inputs produces a
    // byte-identical script (acceptance #11).
    let out2 = coral_test::emit_k6(std::slice::from_ref(&case), &spec);
    assert_eq!(out.script, out2.script, "emit must be byte-deterministic");

    assert_eq!(out.included, 1);
    // Method translation (acceptance #5).
    assert!(out.script.contains("http.get(`${SVC_API_BASE}/users`"));
    assert!(out.script.contains("http.post(`${SVC_API_BASE}/users`"));
    // Body mapping: JSON.stringify with Content-Type.
    assert!(out.script.contains("JSON.stringify"));
    assert!(out.script.contains("'Content-Type': 'application/json'"));
    // Status check + body_contains check.
    assert!(out.script.contains("(r) => r.status === 200"));
    assert!(out.script.contains("(r) => r.status === 201"));
    assert!(out.script.contains("(r) => r.body.includes('alice')"));
    // Inter-case `sleep(1)`.
    assert!(out.script.contains("sleep(1);"));
    // Footer summary present.
    assert!(out.script.contains("included=1 skipped=0"));
}

#[test]
fn emit_k6_skips_exec_steps_with_comment() {
    let yaml = r#"
name: mixed
service: api
steps:
  - http: GET /healthz
    expect: { status: 200 }
  - exec: ["echo", "ok"]
    expect: { exit_code: 0 }
"#;
    let suite: YamlSuite = serde_yaml_ng::from_str(yaml).unwrap();
    let case = user_defined_case("user-defined:mixed", suite);
    let spec = spec_with_service("api", 3000, None);

    let out = coral_test::emit_k6(&[case], &spec);
    // The HTTP step is included; the exec step gets a SKIPPED comment
    // inline.
    assert_eq!(out.included, 1);
    assert!(out.script.contains("http.get(`${SVC_API_BASE}/healthz`"));
    assert!(
        out.script
            .contains("// SKIPPED user-defined:mixed:1 — exec step not k6-compatible"),
        "got: {}",
        out.script
    );
}

#[test]
fn emit_k6_healthcheck_tcp_skipped() {
    let hc = Healthcheck {
        kind: HealthcheckKind::Tcp { port: 5432 },
        timing: HealthcheckTiming::default(),
    };
    let spec = spec_with_service("db", 5432, Some(hc));
    let case = TestCase {
        id: "healthcheck:db".into(),
        name: "db healthcheck".into(),
        kind: TestKind::Healthcheck,
        service: Some("db".into()),
        tags: vec!["healthcheck".into(), "smoke".into()],
        source: TestSource::Inline,
        spec: TestSpec::empty(),
    };
    let out = coral_test::emit_k6(&[case], &spec);
    // The case is skipped end-to-end.
    assert_eq!(out.included, 0);
    assert_eq!(out.skipped.len(), 1);
    let note = &out.skipped[0];
    assert_eq!(note.case_id, "healthcheck:db");
    // Acceptance #6: SKIPPED comment present in script.
    assert!(
        out.script
            .contains("// SKIPPED healthcheck:db: healthcheck kind 'tcp'"),
        "got: {}",
        out.script
    );
    // Footer reflects the skip count.
    assert!(out.script.contains("included=0 skipped=1"));
}

#[test]
fn emit_k6_service_base_uses_declared_port() {
    // Acceptance #4: declared port (not allocated host port) flows
    // into `SVC_<NAME>_BASE`.
    let spec = spec_with_service("worker", 9090, None);
    let out = coral_test::emit_k6(&[], &spec);
    assert!(
        out.script.contains(
            "const SVC_WORKER_BASE = __ENV.CORAL_WORKER_BASE || 'http://localhost:9090';"
        ),
        "got: {}",
        out.script
    );
}

#[test]
#[allow(non_snake_case)]
fn emit_k6_unknown_service_falls_back_to_BASE_with_todo() {
    // D4 fallback: case targets a service the EnvironmentSpec doesn't
    // know about. Emitter falls back to `${BASE}` and drops a TODO.
    let yaml = r#"
name: orphan
service: ghost
steps:
  - http: GET /x
    expect: { status: 200 }
"#;
    let suite: YamlSuite = serde_yaml_ng::from_str(yaml).unwrap();
    let case = user_defined_case("user-defined:orphan", suite);
    let spec = spec_with_service("api", 3000, None);

    let out = coral_test::emit_k6(&[case], &spec);
    assert_eq!(out.included, 1);
    assert!(
        out.script
            .contains("// TODO user-defined:orphan:0 — service 'ghost' not declared"),
        "got: {}",
        out.script
    );
    assert!(
        out.script.contains("http.get(`${BASE}/x`"),
        "got: {}",
        out.script
    );
}
