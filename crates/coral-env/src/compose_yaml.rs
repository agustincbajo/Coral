//! Render an `EnvPlan` to a Docker Compose YAML string.
//!
//! v0.17 wave 2 covers the schema fields the wave-1 `EnvironmentSpec`
//! exposes: `image`, `build { context, dockerfile, target, args,
//! cache_from, cache_to }`, `ports`, `env`, `depends_on`,
//! `healthcheck`. `develop.watch` (compose 2.22+) follows in wave 3
//! once the rebuild/restart interaction with the healthcheck loop is
//! pinned by the integration test (see PRD risk #6).

use crate::plan::{EnvPlan, ServiceSpecPlan};
use crate::spec::{Healthcheck, HealthcheckKind, HealthcheckTiming, RealService, ServiceKind};
use serde_yaml_ng::Value;

/// Render the plan as a YAML document compatible with `docker compose
/// up -f <out>`. Stable ordering: services map is `BTreeMap` so the
/// output is byte-stable per plan, which makes the artifact-hash
/// comparison in `EnvBackend::up()` reliable.
pub fn render(plan: &EnvPlan) -> String {
    let mut services_yaml = serde_yaml_ng::Mapping::new();
    for (name, service) in &plan.services {
        let body = render_service(name, service, plan);
        services_yaml.insert(Value::String(name.clone()), body);
    }

    let mut top = serde_yaml_ng::Mapping::new();
    top.insert(
        Value::String("name".into()),
        Value::String(plan.project_name.clone()),
    );
    top.insert(
        Value::String("services".into()),
        Value::Mapping(services_yaml),
    );

    let document = Value::Mapping(top);
    serde_yaml_ng::to_string(&document).unwrap_or_else(|_| String::new())
}

fn render_service(name: &str, service: &ServiceSpecPlan, plan: &EnvPlan) -> Value {
    let mut out = serde_yaml_ng::Mapping::new();
    out.insert(
        Value::String("container_name".into()),
        Value::String(format!("{}-{}", plan.project_name, name)),
    );

    // Apply the environment-level `env_file` to every service so the
    // declared envs reach all containers (compose's idiomatic pattern).
    // Per-service `env: { K = V }` overrides still take precedence
    // because compose merges service-level `environment` after
    // `env_file`.
    if let Some(env_file) = &plan.env_file {
        out.insert(
            Value::String("env_file".into()),
            Value::Sequence(vec![Value::String(env_file.to_string_lossy().into_owned())]),
        );
    }

    match &service.kind {
        ServiceKind::Real(real) => render_real(&mut out, real, service),
        ServiceKind::Mock(_) => {
            // v0.18 will wire mock servers (Mockoon/WireMock/Hoverfly) by
            // launching their official containers; v0.17 leaves the entry
            // empty so the YAML still parses.
            out.insert(
                Value::String("image".into()),
                Value::String("ghcr.io/agustincbajo/coral-mock-placeholder:latest".into()),
            );
        }
    }

    Value::Mapping(out)
}

fn render_real(out: &mut serde_yaml_ng::Mapping, real: &RealService, plan: &ServiceSpecPlan) {
    if let Some(image) = &real.image {
        out.insert(Value::String("image".into()), Value::String(image.clone()));
    }
    if let Some(build) = &real.build {
        let mut bmap = serde_yaml_ng::Mapping::new();
        let context = plan
            .resolved_context
            .clone()
            .map(|p| p.join(&build.context))
            .unwrap_or_else(|| build.context.clone());
        bmap.insert(
            Value::String("context".into()),
            Value::String(context.to_string_lossy().into_owned()),
        );
        if let Some(dockerfile) = &build.dockerfile {
            bmap.insert(
                Value::String("dockerfile".into()),
                Value::String(dockerfile.to_string_lossy().into_owned()),
            );
        }
        if let Some(target) = &build.target {
            bmap.insert(
                Value::String("target".into()),
                Value::String(target.clone()),
            );
        }
        if !build.args.is_empty() {
            let mut args = serde_yaml_ng::Mapping::new();
            for (k, v) in &build.args {
                args.insert(Value::String(k.clone()), Value::String(v.clone()));
            }
            out.insert(Value::String("args".into()), Value::Mapping(args));
        }
        if !build.cache_from.is_empty() {
            bmap.insert(
                Value::String("cache_from".into()),
                Value::Sequence(
                    build
                        .cache_from
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        if let Some(cache_to) = &build.cache_to {
            bmap.insert(
                Value::String("cache_to".into()),
                Value::Sequence(vec![Value::String(cache_to.clone())]),
            );
        }
        out.insert(Value::String("build".into()), Value::Mapping(bmap));
    }
    if !real.ports.is_empty() {
        out.insert(
            Value::String("ports".into()),
            Value::Sequence(
                real.ports
                    .iter()
                    .map(|p| Value::String(format!("{p}:{p}")))
                    .collect(),
            ),
        );
    }
    if !real.env.is_empty() {
        out.insert(
            Value::String("environment".into()),
            Value::Sequence(
                real.env
                    .iter()
                    .map(|(k, v)| Value::String(format!("{k}={v}")))
                    .collect(),
            ),
        );
    }
    if !real.depends_on.is_empty() {
        // depends_on supports the long-form `condition: service_healthy`
        // dict in compose 2.x; we always emit it because the EnvBackend
        // wait-for-healthy loop depends on healthchecks anyway.
        let mut deps = serde_yaml_ng::Mapping::new();
        for dep in &real.depends_on {
            let mut cond = serde_yaml_ng::Mapping::new();
            cond.insert(
                Value::String("condition".into()),
                Value::String("service_healthy".into()),
            );
            deps.insert(Value::String(dep.clone()), Value::Mapping(cond));
        }
        out.insert(Value::String("depends_on".into()), Value::Mapping(deps));
    }
    if let Some(hc) = &real.healthcheck {
        out.insert(Value::String("healthcheck".into()), render_healthcheck(hc));
    }
}

fn render_healthcheck(hc: &Healthcheck) -> Value {
    let mut out = serde_yaml_ng::Mapping::new();
    let test = match &hc.kind {
        HealthcheckKind::Http {
            path,
            expect_status,
            headers,
        } => {
            // curl-based probe: portable across alpine/debian containers
            // that ship curl. Users with images that don't have curl can
            // override with `kind = "exec"`. We bake the expected status
            // into the test rather than parse `--write-out` output.
            //
            // Render any declared headers as `-H 'k: v'` flags so probes
            // against authenticated endpoints succeed. Without this the
            // probe would silently get 401/403 and the service would
            // report unhealthy forever.
            let mut header_args = String::new();
            for (k, v) in headers {
                // Single-quote the value because the whole CMD-SHELL is
                // already double-quoted by YAML scalar rules; embedded
                // single quotes in a header value are vanishingly rare.
                header_args.push_str(&format!(" -H '{k}: {v}'"));
            }
            vec![
                Value::String("CMD-SHELL".into()),
                Value::String(format!(
                    "curl -fsS -o /dev/null -w '%{{http_code}}'{header_args} http://localhost:8080{path} | grep -q '^{expect_status}$' || exit 1"
                )),
            ]
        }
        HealthcheckKind::Tcp { port } => vec![
            Value::String("CMD-SHELL".into()),
            Value::String(format!(
                "nc -z localhost {port} || (echo > /dev/tcp/localhost/{port}) >/dev/null 2>&1"
            )),
        ],
        HealthcheckKind::Exec { cmd } => {
            let mut tokens = vec![Value::String("CMD".into())];
            for arg in cmd {
                tokens.push(Value::String(arg.clone()));
            }
            tokens
        }
        HealthcheckKind::Grpc { port, service } => {
            let svc_arg = service
                .as_ref()
                .map(|s| format!(" -service {s}"))
                .unwrap_or_default();
            vec![
                Value::String("CMD-SHELL".into()),
                Value::String(format!(
                    "grpc_health_probe -addr=:{port}{svc_arg} || exit 1"
                )),
            ]
        }
    };
    out.insert(Value::String("test".into()), Value::Sequence(test));
    let HealthcheckTiming {
        interval_s,
        timeout_s,
        retries,
        start_period_s,
        start_interval_s,
        ..
    } = hc.timing;
    out.insert(
        Value::String("interval".into()),
        Value::String(format!("{interval_s}s")),
    );
    out.insert(
        Value::String("timeout".into()),
        Value::String(format!("{timeout_s}s")),
    );
    out.insert(
        Value::String("retries".into()),
        Value::Number(serde_yaml_ng::Number::from(retries as i64)),
    );
    out.insert(
        Value::String("start_period".into()),
        Value::String(format!("{start_period_s}s")),
    );
    if let Some(si) = start_interval_s {
        out.insert(
            Value::String("start_interval".into()),
            Value::String(format!("{si}s")),
        );
    }
    Value::Mapping(out)
}

/// Convenience: 8-char content hash of the rendered YAML, used to
/// detect drift in `EnvBackend::up()` and to name the rendered file
/// (`<hash>.yml`).
pub fn content_hash(yaml: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in yaml.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:08x}", hash & 0xffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{BuildSpec, HealthcheckKind, RealService};
    use std::collections::BTreeMap;

    fn empty_plan(name: &str) -> EnvPlan {
        EnvPlan {
            name: name.into(),
            project_name: format!("coral-{name}-deadbeef"),
            mode: crate::spec::EnvMode::Managed,
            services: BTreeMap::new(),
            env_file: None,
            project_root: std::path::PathBuf::from("/tmp"),
        }
    }

    fn real_service(name: &str) -> ServiceSpecPlan {
        ServiceSpecPlan {
            name: name.into(),
            kind: ServiceKind::Real(Box::new(RealService {
                repo: None,
                image: Some("postgres:16".into()),
                build: None,
                ports: vec![5432],
                env: BTreeMap::from([("POSTGRES_PASSWORD".into(), "dev".into())]),
                depends_on: vec![],
                healthcheck: Some(Healthcheck {
                    kind: HealthcheckKind::Tcp { port: 5432 },
                    timing: HealthcheckTiming::default(),
                }),
                watch: None,
            })),
            resolved_context: None,
        }
    }

    #[test]
    fn render_emits_project_name_and_services_section() {
        let mut plan = empty_plan("dev");
        plan.services.insert("db".into(), real_service("db"));
        let yaml = render(&plan);
        assert!(yaml.contains("name: coral-dev-deadbeef"));
        assert!(yaml.contains("services:"));
        assert!(yaml.contains("db:"));
        assert!(yaml.contains("image: postgres:16"));
    }

    #[test]
    fn render_includes_healthcheck_with_timing() {
        let mut plan = empty_plan("dev");
        plan.services.insert("db".into(), real_service("db"));
        let yaml = render(&plan);
        assert!(yaml.contains("healthcheck:"));
        assert!(yaml.contains("interval:"));
        assert!(yaml.contains("retries: 5"));
    }

    #[test]
    fn render_emits_depends_on_with_service_healthy() {
        let mut plan = empty_plan("dev");
        let mut api = real_service("api");
        if let ServiceKind::Real(real) = &mut api.kind {
            real.depends_on = vec!["db".into()];
        }
        plan.services.insert("api".into(), api);
        let yaml = render(&plan);
        assert!(yaml.contains("depends_on:"));
        assert!(yaml.contains("condition: service_healthy"));
    }

    #[test]
    fn render_with_build_emits_context_and_target() {
        let mut plan = empty_plan("dev");
        let svc = ServiceSpecPlan {
            name: "api".into(),
            kind: ServiceKind::Real(Box::new(RealService {
                repo: Some("api".into()),
                image: None,
                build: Some(BuildSpec {
                    context: std::path::PathBuf::from("."),
                    dockerfile: Some(std::path::PathBuf::from("Dockerfile")),
                    target: Some("dev".into()),
                    cache_from: vec![],
                    cache_to: None,
                    args: BTreeMap::new(),
                }),
                ports: vec![3000],
                env: BTreeMap::new(),
                depends_on: vec![],
                healthcheck: None,
                watch: None,
            })),
            resolved_context: Some(std::path::PathBuf::from("/work/repos/api")),
        };
        plan.services.insert("api".into(), svc);
        let yaml = render(&plan);
        assert!(yaml.contains("build:"));
        assert!(yaml.contains("/work/repos/api"));
        assert!(yaml.contains("target: dev"));
    }

    #[test]
    fn content_hash_is_stable() {
        let h1 = content_hash("hello");
        let h2 = content_hash("hello");
        assert_eq!(h1, h2);
        assert_ne!(content_hash("hello"), content_hash("world"));
        assert_eq!(h1.len(), 8);
    }

    #[test]
    fn render_propagates_env_file_to_every_service() {
        let mut plan = empty_plan("dev");
        plan.env_file = Some(std::path::PathBuf::from("env/dev.env"));
        plan.services.insert("db".into(), real_service("db"));
        plan.services.insert("api".into(), real_service("api"));
        let yaml = render(&plan);
        // Every service inherits env_file: [env/dev.env]. Compose merges
        // env_file before per-service `environment:` so per-service env
        // overrides still win.
        let env_file_count = yaml.matches("env/dev.env").count();
        assert!(
            env_file_count >= 2,
            "expected env_file to fan out across all services, got: {yaml}"
        );
    }

    #[test]
    fn render_omits_env_file_when_unset() {
        let mut plan = empty_plan("dev");
        plan.services.insert("db".into(), real_service("db"));
        let yaml = render(&plan);
        assert!(!yaml.contains("env_file"));
    }

    #[test]
    fn render_http_healthcheck_emits_header_flags() {
        let mut plan = empty_plan("dev");
        let mut svc = real_service("api");
        if let ServiceKind::Real(real) = &mut svc.kind {
            real.healthcheck = Some(Healthcheck {
                kind: HealthcheckKind::Http {
                    path: "/health".into(),
                    expect_status: 200,
                    headers: BTreeMap::from([(
                        "X-Internal-Auth".into(),
                        "${HEALTHCHECK_TOKEN}".into(),
                    )]),
                },
                timing: HealthcheckTiming::default(),
            });
        }
        plan.services.insert("api".into(), svc);
        let yaml = render(&plan);
        // YAML single-quote escaping turns `'foo'` into `''foo''` inside
        // the surrounding single-quoted scalar; assert the header field
        // name and value travel through, regardless of escape form.
        assert!(
            yaml.contains("-H ") && yaml.contains("X-Internal-Auth: ${HEALTHCHECK_TOKEN}"),
            "expected curl probe to render auth header flag, got:\n{yaml}"
        );
    }

    #[test]
    fn render_http_healthcheck_without_headers_is_clean() {
        let mut plan = empty_plan("dev");
        let mut svc = real_service("api");
        if let ServiceKind::Real(real) = &mut svc.kind {
            real.healthcheck = Some(Healthcheck {
                kind: HealthcheckKind::Http {
                    path: "/health".into(),
                    expect_status: 200,
                    headers: BTreeMap::new(),
                },
                timing: HealthcheckTiming::default(),
            });
        }
        plan.services.insert("api".into(), svc);
        let yaml = render(&plan);
        assert!(yaml.contains("curl -fsS"));
        // No stray `-H ` flag when no headers were declared.
        assert!(!yaml.contains(" -H "));
    }

    #[test]
    fn render_grpc_healthcheck_emits_grpc_health_probe() {
        let mut plan = empty_plan("dev");
        let mut svc = real_service("api");
        if let ServiceKind::Real(real) = &mut svc.kind {
            real.healthcheck = Some(Healthcheck {
                kind: HealthcheckKind::Grpc {
                    port: 50051,
                    service: Some("health.v1".into()),
                },
                timing: HealthcheckTiming::default(),
            });
        }
        plan.services.insert("api".into(), svc);
        let yaml = render(&plan);
        assert!(yaml.contains("grpc_health_probe -addr=:50051 -service health.v1"));
    }

    #[test]
    fn render_is_deterministic_for_identical_plans() {
        // The content hash drives `.coral/env/compose/<hash>.yml` and
        // drift detection; if rendering ever became non-deterministic
        // (e.g. iterating a HashMap), `coral up` would re-render every
        // invocation. Pin the property explicitly.
        let mut plan = empty_plan("dev");
        plan.services.insert("api".into(), real_service("api"));
        plan.services.insert("db".into(), real_service("db"));
        let a = render(&plan);
        let b = render(&plan);
        assert_eq!(a, b);
        assert_eq!(content_hash(&a), content_hash(&b));
    }
}
