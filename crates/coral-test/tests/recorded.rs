//! Integration tests for `RecordedRunner` (v0.23.2).
//!
//! Exercise the runner end-to-end against a tiny TCP listener that
//! impersonates the live service. The capture-side (`coral test
//! record`) is Linux-only and has its own gating tests in the CLI
//! crate; this file exercises the parser + replay (always-on, runs
//! on macOS).
//!
//! These tests pin the v0.23.2 acceptance criteria:
//! - **#3**: `RecordedRunner` parses Keploy YAML schema (parser-only).
//! - **#4**: `coral test --kind recorded --service api` replays each
//!   captured exchange and emits a `TestReport` per case (the
//!   integration variant; CLI-level smoke is in `coral-cli/tests/`).
//! - **#5**: Status code mismatch → Fail.
//! - **#6**: Body diff with `ignore_response_fields` filtering matches → Pass.
//!
//! Why not `wiremock`/`httpmock`: zero-new-deps mandate. We hand-roll
//! a single-shot TCP listener (mirror of the chaos integration test
//! pattern) so the test surface stays in the standard library.

use coral_env::spec::EnvMode;
use coral_env::{
    EnvBackend, EnvPlan, EnvironmentSpec, MockBackend, RealService, RecordedConfig, ServiceKind,
};
use coral_test::recorded_runner::{KeployRequest, KeployResponse, KeploySpec, KeployTestCase};
use coral_test::{RecordedRunner, TestKind, TestRunner, TestStatus};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;

/// Build a minimal `EnvironmentSpec` with a `[recorded]` block.
fn spec_with_recorded(ignore_fields: Vec<String>) -> EnvironmentSpec {
    let mut services = BTreeMap::new();
    services.insert(
        "api".to_string(),
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
        recorded: Some(RecordedConfig {
            ignore_response_fields: ignore_fields,
        }),
    }
}

/// Bind a single-shot TCP listener that responds with the given HTTP
/// payload, returning `(port, join_handle)`. The handle joins on test
/// completion.
fn one_shot_http_server(response: String) -> (u16, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        // Read the request (we don't care about its content; the
        // canned response goes back regardless).
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(250)))
            .ok();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        stream.write_all(response.as_bytes()).ok();
    });
    (port, handle)
}

/// Write a Keploy YAML to `<root>/.coral/tests/recorded/<service>/<name>.yaml`.
fn write_recorded_yaml(root: &Path, service: &str, name: &str, kc: &KeployTestCase) {
    let dir = root.join(".coral/tests/recorded").join(service);
    std::fs::create_dir_all(&dir).unwrap();
    let yaml = serde_yaml_ng::to_string(kc).unwrap();
    std::fs::write(dir.join(format!("{name}.yaml")), yaml).unwrap();
}

/// Acceptance criterion #4 + #6 — replay against a live (mock)
/// endpoint with `ignore_response_fields = ["id"]` passes when only
/// the `id` field differs.
#[test]
fn recorded_replay_with_ignore_fields_passes_against_mock_server() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 24\r\n\r\n{\"id\":99,\"name\":\"alice\"}".to_string();
    let (port, handle) = one_shot_http_server(response);

    let kc = KeployTestCase {
        version: "api.keploy.io/v1beta1".into(),
        kind: "Http".into(),
        name: "get-user".into(),
        spec: KeploySpec {
            req: KeployRequest {
                method: "GET".into(),
                url: format!("http://127.0.0.1:{port}/users/42"),
                header: BTreeMap::new(),
                body: String::new(),
            },
            resp: KeployResponse {
                status_code: 200,
                header: BTreeMap::from([("Content-Type".into(), "application/json".into())]),
                // Captured `id=42`; the server returns `id=99`.
                body: r#"{"id":42,"name":"alice"}"#.into(),
            },
        },
    };

    let tmp = tempfile::TempDir::new().unwrap();
    write_recorded_yaml(tmp.path(), "api", "get-user", &kc);

    let pairs = coral_test::discover_recorded(tmp.path()).expect("discover");
    assert_eq!(pairs.len(), 1);
    let case = &pairs[0].0;
    assert_eq!(case.kind, TestKind::Recorded);
    assert_eq!(case.service.as_deref(), Some("api"));

    let spec = spec_with_recorded(vec!["id".to_string()]);
    let backend: Arc<dyn EnvBackend> = Arc::new(MockBackend::new());
    let plan = EnvPlan::from_spec(&spec, tmp.path(), &BTreeMap::new()).expect("plan");
    let env_handle = coral_env::EnvHandle {
        backend: backend.name().to_string(),
        artifact_hash: "test".into(),
        artifact_path: plan.project_root.join(".coral/env/compose/test.yml"),
        state: BTreeMap::new(),
    };
    let runner = RecordedRunner::new(backend, plan, spec);
    let report = runner.run(case, &env_handle).expect("run");
    handle.join().expect("server join");
    assert!(
        matches!(report.status, TestStatus::Pass),
        "expected Pass, got {:?}",
        report.status
    );
    // Evidence: HTTP details captured.
    let http = report.evidence.http.expect("http evidence");
    assert_eq!(http.method, "GET");
    assert_eq!(http.status, 200);
}

/// Acceptance criterion #5 — status code mismatch.
#[test]
fn recorded_replay_status_mismatch_fails_against_mock_server() {
    // Server returns 500; captured says 200.
    let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: 14\r\n\r\n{\"err\":\"boom\"}".to_string();
    let (port, handle) = one_shot_http_server(response);

    let kc = KeployTestCase {
        version: "v1beta1".into(),
        kind: "Http".into(),
        name: "get-user".into(),
        spec: KeploySpec {
            req: KeployRequest {
                method: "GET".into(),
                url: format!("http://127.0.0.1:{port}/u"),
                header: BTreeMap::new(),
                body: String::new(),
            },
            resp: KeployResponse {
                status_code: 200,
                header: BTreeMap::from([("Content-Type".into(), "application/json".into())]),
                body: r#"{"id":1}"#.into(),
            },
        },
    };

    let tmp = tempfile::TempDir::new().unwrap();
    write_recorded_yaml(tmp.path(), "api", "get-user", &kc);

    let pairs = coral_test::discover_recorded(tmp.path()).expect("discover");
    let case = &pairs[0].0;

    let spec = spec_with_recorded(vec![]);
    let backend: Arc<dyn EnvBackend> = Arc::new(MockBackend::new());
    let plan = EnvPlan::from_spec(&spec, tmp.path(), &BTreeMap::new()).expect("plan");
    let env_handle = coral_env::EnvHandle {
        backend: backend.name().to_string(),
        artifact_hash: "test".into(),
        artifact_path: plan.project_root.join(".coral/env/compose/test.yml"),
        state: BTreeMap::new(),
    };
    let runner = RecordedRunner::new(backend, plan, spec);
    let report = runner.run(case, &env_handle).expect("run");
    handle.join().expect("server join");
    match &report.status {
        TestStatus::Fail { reason } => {
            assert!(reason.contains("status mismatch"), "msg: {reason}");
            assert!(reason.contains("200"), "msg: {reason}");
            assert!(reason.contains("500"), "msg: {reason}");
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

/// Body diff WITHOUT ignore_fields: replay should fail when the live
/// body differs from the captured one.
#[test]
fn recorded_replay_body_diff_without_ignore_fields_fails_against_mock_server() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 24\r\n\r\n{\"id\":99,\"name\":\"alice\"}".to_string();
    let (port, handle) = one_shot_http_server(response);

    let kc = KeployTestCase {
        version: "v1beta1".into(),
        kind: "Http".into(),
        name: "get-user".into(),
        spec: KeploySpec {
            req: KeployRequest {
                method: "GET".into(),
                url: format!("http://127.0.0.1:{port}/u"),
                header: BTreeMap::new(),
                body: String::new(),
            },
            resp: KeployResponse {
                status_code: 200,
                header: BTreeMap::from([("Content-Type".into(), "application/json".into())]),
                body: r#"{"id":42,"name":"alice"}"#.into(),
            },
        },
    };

    let tmp = tempfile::TempDir::new().unwrap();
    write_recorded_yaml(tmp.path(), "api", "get-user", &kc);

    let pairs = coral_test::discover_recorded(tmp.path()).expect("discover");
    let case = &pairs[0].0;

    let spec = spec_with_recorded(vec![]); // no ignore list
    let backend: Arc<dyn EnvBackend> = Arc::new(MockBackend::new());
    let plan = EnvPlan::from_spec(&spec, tmp.path(), &BTreeMap::new()).expect("plan");
    let env_handle = coral_env::EnvHandle {
        backend: backend.name().to_string(),
        artifact_hash: "test".into(),
        artifact_path: plan.project_root.join(".coral/env/compose/test.yml"),
        state: BTreeMap::new(),
    };
    let runner = RecordedRunner::new(backend, plan, spec);
    let report = runner.run(case, &env_handle).expect("run");
    handle.join().expect("server join");
    assert!(
        matches!(report.status, TestStatus::Fail { .. }),
        "expected Fail, got {:?}",
        report.status
    );
}

/// Acceptance criterion #4 (orchestrator integration) — when the
/// `coral test --kind recorded` flag is set, the orchestrator picks
/// up captured cases from `.coral/tests/recorded/`.
#[test]
fn run_test_suite_filtered_picks_up_recorded_when_kind_recorded() {
    // Single-shot mock: the runner replays one case against this server.
    let response =
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 8\r\n\r\n{\"x\":1}"
            .to_string();
    let (port, handle) = one_shot_http_server(response);

    let kc = KeployTestCase {
        version: "v1beta1".into(),
        kind: "Http".into(),
        name: "x".into(),
        spec: KeploySpec {
            req: KeployRequest {
                method: "GET".into(),
                url: format!("http://127.0.0.1:{port}/x"),
                header: BTreeMap::new(),
                body: String::new(),
            },
            resp: KeployResponse {
                status_code: 200,
                header: BTreeMap::from([("Content-Type".into(), "application/json".into())]),
                body: r#"{"x":1}"#.into(),
            },
        },
    };

    let tmp = tempfile::TempDir::new().unwrap();
    write_recorded_yaml(tmp.path(), "api", "x", &kc);

    let spec = spec_with_recorded(vec![]);
    let backend: Arc<dyn EnvBackend> = Arc::new(MockBackend::new());
    let plan = EnvPlan::from_spec(&spec, tmp.path(), &BTreeMap::new()).expect("plan");
    let env_handle = coral_env::EnvHandle {
        backend: backend.name().to_string(),
        artifact_hash: "test".into(),
        artifact_path: plan.project_root.join(".coral/env/compose/test.yml"),
        state: BTreeMap::new(),
    };

    let filters = coral_test::TestFilters {
        services: Vec::new(),
        tags: Vec::new(),
        kinds: vec![TestKind::Recorded],
        include_discovered: false,
    };
    let reports = coral_test::run_test_suite_filtered(
        tmp.path(),
        &spec,
        backend,
        &plan,
        &env_handle,
        &filters,
        false,
    )
    .expect("run");
    handle.join().expect("server join");
    assert_eq!(reports.len(), 1);
    assert!(
        matches!(reports[0].status, TestStatus::Pass),
        "expected Pass, got {:?}",
        reports[0].status
    );
    assert_eq!(reports[0].case.kind, TestKind::Recorded);
}

/// Acceptance criterion #4 (default-kinds gate) — without
/// `--kind recorded` (empty kinds list), recorded cases are NOT
/// included so pre-v0.23.2 invocations are byte-compatible.
#[test]
fn run_test_suite_filtered_skips_recorded_when_kind_unspecified() {
    let kc = KeployTestCase {
        version: "v1beta1".into(),
        kind: "Http".into(),
        name: "x".into(),
        spec: KeploySpec {
            req: KeployRequest {
                method: "GET".into(),
                url: "http://127.0.0.1:1/x".into(), // never invoked
                header: BTreeMap::new(),
                body: String::new(),
            },
            resp: KeployResponse {
                status_code: 200,
                header: BTreeMap::new(),
                body: r#"{}"#.into(),
            },
        },
    };

    let tmp = tempfile::TempDir::new().unwrap();
    write_recorded_yaml(tmp.path(), "api", "x", &kc);

    let spec = spec_with_recorded(vec![]);
    let backend: Arc<dyn EnvBackend> = Arc::new(MockBackend::new());
    let plan = EnvPlan::from_spec(&spec, tmp.path(), &BTreeMap::new()).expect("plan");
    let env_handle = coral_env::EnvHandle {
        backend: backend.name().to_string(),
        artifact_hash: "test".into(),
        artifact_path: plan.project_root.join(".coral/env/compose/test.yml"),
        state: BTreeMap::new(),
    };

    let filters = coral_test::TestFilters {
        services: Vec::new(),
        tags: Vec::new(),
        kinds: Vec::new(), // no --kind flag → recorded NOT included
        include_discovered: false,
    };
    let reports = coral_test::run_test_suite_filtered(
        tmp.path(),
        &spec,
        backend,
        &plan,
        &env_handle,
        &filters,
        false,
    )
    .expect("run");
    // No healthcheck declared, no user-defined YAMLs, recorded gated:
    // the suite is empty.
    assert!(reports.is_empty(), "got: {reports:?}");
}

/// Suppress unused import warning when a future test reaches into the
/// channel for assertion. The mpsc helper is already imported because
/// the patterns from `chaos.rs` use it; reserving it here.
#[allow(dead_code)]
fn _channel_lifetime_hint() -> (mpsc::Sender<()>, mpsc::Receiver<()>) {
    mpsc::channel()
}
